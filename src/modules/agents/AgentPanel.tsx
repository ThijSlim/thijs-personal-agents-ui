import { useEffect, useId, useRef, useState } from "react";
import * as Tabs from "@radix-ui/react-tabs";
import { approve, deny, markSeen, readConversation, sendReply } from "../../core/ipc";
import { StateDot } from "../../core/StateDot";
import type { Agent, Conversation as ConversationData, UiState } from "../../core/types";
import { Conversation } from "./Conversation";
import { TerminalView } from "./TerminalView";

/** Re-read the conversation at most this often while state events stream in. */
const READ_INTERVAL_MS = 500;
/** While the agent is working (and the conversation is on screen), re-read this often. */
export const POLL_MS = 1500;

function useDocumentVisible(): boolean {
  const [visible, setVisible] = useState(() => document.visibilityState === "visible");
  useEffect(() => {
    const update = () => setVisible(document.visibilityState === "visible");
    document.addEventListener("visibilitychange", update);
    return () => document.removeEventListener("visibilitychange", update);
  }, []);
  return visible;
}

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));

function findAgent(state: UiState, paneId: string): { agent: Agent; spaceLabel: string } | null {
  for (const space of state.spaces) {
    const agent = space.agents.find((a) => a.paneId === paneId);
    if (agent) return { agent, spaceLabel: space.label };
  }
  return null;
}

interface Props {
  paneId: string;
  state: UiState;
}

export function AgentPanel({ paneId, state }: Props) {
  const titleId = useId();
  const replyId = useId();
  const [conv, setConv] = useState<ConversationData | null>(null);
  const [readError, setReadError] = useState<string | null>(null);
  const [actionError, setActionError] = useState<string | null>(null);
  const [reply, setReply] = useState("");
  const [sending, setSending] = useState(false);
  const [answering, setAnswering] = useState(false);
  const [tab, setTab] = useState("conversation");
  const visible = useDocumentVisible();

  // Header info survives the agent leaving the model (session ended).
  const found = findAgent(state, paneId);
  const lastFound = useRef<{ paneId: string; info: typeof found }>({ paneId, info: found });
  if (found || lastFound.current.paneId !== paneId) lastFound.current = { paneId, info: found };
  const info = lastFound.current.info;

  const paneRef = useRef(paneId);
  paneRef.current = paneId;
  const lastRead = useRef(0);
  const timer = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);
  const seq = useRef(0);
  // Seq of the last read started before an Approve/Deny press; only reads
  // after it may re-enable the buttons. `null` when not answering.
  const answeredAt = useRef<number | null>(null);

  const read = () => {
    lastRead.current = Date.now();
    const mine = ++seq.current;
    const pane = paneRef.current;
    readConversation(pane)
      .then((c) => {
        if (mine !== seq.current) return;
        // A gone pane has no text left; keep what we last showed.
        setConv((prev) => (c.ended && !c.text && prev ? { ...c, text: prev.text } : c));
        setReadError(null);
        if (answeredAt.current !== null && mine > answeredAt.current) {
          answeredAt.current = null;
          setAnswering(false);
        }
      })
      .catch((e) => {
        if (mine !== seq.current) return;
        console.error("read_conversation failed", e);
        setReadError(`Couldn't read the conversation: ${message(e)}`);
      });
  };

  const clearTimer = () => {
    clearTimeout(timer.current);
    timer.current = undefined;
  };

  // Opening a pane: mark it seen, reset, and read right away.
  useEffect(() => {
    markSeen(paneId).catch((e) => console.error("mark_seen failed", e));
    setConv(null);
    setReadError(null);
    setActionError(null);
    setReply("");
    setAnswering(false);
    answeredAt.current = null;
    read();
    return () => {
      clearTimer();
      seq.current++; // drop any in-flight read for this pane
    };
  }, [paneId]);

  /** Read now, or once READ_INTERVAL_MS has passed since the last read. */
  const requestRead = () => {
    const wait = lastRead.current + READ_INTERVAL_MS - Date.now();
    if (wait <= 0) {
      clearTimer();
      read();
    } else if (timer.current === undefined) {
      timer.current = setTimeout(() => {
        timer.current = undefined;
        read();
      }, wait);
    }
  };

  // Any change to this pane's agent (status, seq, title, gone) re-reads, throttled.
  const agentKey = JSON.stringify(found?.agent ?? null);
  const seenKey = useRef({ paneId, agentKey });
  useEffect(() => {
    const prev = seenKey.current;
    seenKey.current = { paneId, agentKey };
    if (prev.paneId !== paneId || prev.agentKey === agentKey) return;
    requestRead();
  }, [paneId, agentKey]);

  // Show progress: poll while the agent works and its conversation is on screen.
  const polling = found?.agent.status === "working" && tab === "conversation" && visible;
  useEffect(() => {
    if (!polling) return;
    const id = setInterval(requestRead, POLL_MS);
    return () => clearInterval(id);
  }, [paneId, polling]);

  const ended = conv?.ended ?? false;
  const permission = conv?.permission ?? null;
  const canSend = !ended && !sending && reply.trim() !== "";

  const submit = async () => {
    if (!canSend) return;
    setSending(true);
    setActionError(null);
    try {
      const sent = reply;
      await sendReply(paneId, sent);
      // Keep anything typed while the reply was in flight.
      setReply((cur) => (cur.startsWith(sent) ? cur.slice(sent.length) : cur));
    } catch (e) {
      setActionError(`Couldn't send the reply: ${message(e)}`);
    } finally {
      setSending(false);
    }
  };

  const answer = async (fn: (id: string) => Promise<void>, verb: string) => {
    setAnswering(true);
    answeredAt.current = seq.current;
    setActionError(null);
    try {
      await fn(paneId);
      read(); // don't wait for a status change that may never come
    } catch (e) {
      answeredAt.current = null;
      setAnswering(false);
      setActionError(`Couldn't ${verb}: ${message(e)}`);
    }
  };

  const title = info?.agent.title?.trim() || "Agent";

  return (
    <section className="agent-panel" aria-labelledby={titleId}>
      <header className="agent-header">
        {info && <p className="agent-space">{info.spaceLabel}</p>}
        <div className="agent-title">
          <h1 id={titleId}>{title}</h1>
          {info && !ended && <StateDot status={info.agent.status} />}
        </div>
      </header>

      <Tabs.Root value={tab} onValueChange={setTab} className="agent-tabs">
        <Tabs.List aria-label="Agent views" className="tab-list">
          <Tabs.Trigger value="conversation" className="tab">
            Conversation
          </Tabs.Trigger>
          <Tabs.Trigger value="terminal" className="tab">
            Terminal
          </Tabs.Trigger>
        </Tabs.List>

        <Tabs.Content value="conversation" className="tab-panel conversation-panel">
          {ended && <p className="notice">Session ended</p>}
          {readError && (
            <p role="alert" className="error">
              {readError}
            </p>
          )}
          <Conversation text={conv?.text ?? ""} />

          {permission && (
            <div className="permission">
              <p className="permission-question">{permission.question}</p>
              <div className="permission-actions">
                <button
                  type="button"
                  className="button primary"
                  disabled={ended || answering}
                  onClick={() => answer(approve, "approve")}
                >
                  Approve
                </button>
                <button
                  type="button"
                  className="button"
                  disabled={ended || answering}
                  onClick={() => answer(deny, "deny")}
                >
                  Deny
                </button>
              </div>
            </div>
          )}

          {actionError && (
            <p role="alert" className="error">
              {actionError}
            </p>
          )}

          <form
            className="reply"
            onSubmit={(e) => {
              e.preventDefault();
              void submit();
            }}
          >
            <label htmlFor={replyId}>Reply</label>
            <textarea
              id={replyId}
              rows={3}
              value={reply}
              disabled={ended}
              aria-keyshortcuts="Enter"
              onChange={(e) => setReply(e.target.value)}
              onKeyDown={(e) => {
                if (e.key !== "Enter" || e.shiftKey || e.nativeEvent.isComposing) return;
                e.preventDefault();
                void submit();
              }}
            />
            <button type="submit" className="button primary" disabled={!canSend}>
              Send
            </button>
          </form>
        </Tabs.Content>

        <Tabs.Content value="terminal" className="tab-panel">
          <TerminalView paneId={paneId} label={info?.agent.title?.trim() || paneId} active={tab === "terminal" && visible} ended={ended} />
        </Tabs.Content>
      </Tabs.Root>

      <div role="status" className="sr-only">
        {permission && !ended ? "Agent needs approval" : ""}
      </div>
    </section>
  );
}
