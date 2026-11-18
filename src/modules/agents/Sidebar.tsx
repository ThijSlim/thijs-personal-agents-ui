import { useEffect, useId, useRef, useState } from "react";
import { markSeen, moveSpace } from "../../core/ipc";
import type { Selection } from "../../core/modules";
import { StateDot } from "../../core/StateDot";
import type { Agent, NeedsYouItem, UiState } from "../../core/types";

interface Props {
  state: UiState;
  selection: Selection;
  onSelect(s: Selection): void;
}

const agentName = (a: Pick<Agent, "title">) => a.title?.trim() || "Agent";

export function Sidebar({ state, selection, onSelect }: Props) {
  const headingId = useId();
  const [announcement, setAnnouncement] = useState("");
  const pending = useRef<{ id: string; label: string } | null>(null);
  const dragId = useRef<string | null>(null);
  const [dragOver, setDragOver] = useState<string | null>(null);
  const listRef = useRef<HTMLUListElement>(null);

  // Announce a keyboard move only once herdr's state update has arrived.
  useEffect(() => {
    const p = pending.current;
    if (!p) return;
    const index = state.spaces.findIndex((s) => s.id === p.id);
    if (index < 0) return;
    pending.current = null;
    setAnnouncement(`Moved ${p.label} to position ${index + 1} of ${state.spaces.length}`);
    listRef.current?.querySelector<HTMLElement>(`[data-space-id="${CSS.escape(p.id)}"] .space-button`)?.focus();
  }, [state.spaces]);

  const selectPane = (paneId: string) => {
    onSelect({ module: "agents", id: paneId });
    markSeen(paneId).catch((e) => console.error("mark_seen failed", e));
  };

  const move = (id: string, index: number) =>
    moveSpace(id, index).catch((e) => {
      pending.current = null;
      console.error("move_space failed", e);
    });

  const onKeyDown = (e: React.KeyboardEvent, index: number) => {
    if (!e.altKey || (e.key !== "ArrowUp" && e.key !== "ArrowDown")) return;
    e.preventDefault();
    const target = index + (e.key === "ArrowUp" ? -1 : 1);
    if (target < 0 || target >= state.spaces.length) return;
    const space = state.spaces[index];
    pending.current = { id: space.id, label: space.label };
    move(space.id, target);
  };

  const isSelected = (id: string) => selection?.module === "agents" && selection.id === id;

  return (
    <div className="sidebar">
      {state.connection === "notRunning" && (
        <div className="banner">
          <p>
            <strong>herdr isn't running</strong>
          </p>
          <p>
            Run <code>herdr</code> in a terminal.
          </p>
          <p className="muted">Start it in a terminal, then this app reconnects automatically.</p>
        </div>
      )}
      {state.connection === "connecting" && <p className="muted">Connecting…</p>}

      {state.needsYou.length > 0 && (
        <section aria-labelledby={headingId} className="needs-you">
          <h2 id={headingId}>Needs you ({state.needsYou.length})</h2>
          <ul className="plain">
            {state.needsYou.map((item) => (
              <NeedsYouRow key={item.paneId} item={item} selected={isSelected(item.paneId)} onPick={selectPane} />
            ))}
          </ul>
        </section>
      )}

      {state.spaces.length > 0 && (
        <div>
          <h2 aria-hidden="true">Spaces</h2>
          <ul className="plain" aria-label="Spaces" ref={listRef}>
            {state.spaces.map((space, i) => (
              <li
                key={space.id}
                role="listitem"
                data-space-id={space.id}
                className={dragOver === space.id ? "space drag-over" : "space"}
                draggable
                onDragStart={(e) => {
                  dragId.current = space.id;
                  e.dataTransfer.effectAllowed = "move";
                  e.dataTransfer.setData("text/plain", space.id);
                }}
                onDragOver={(e) => {
                  if (!dragId.current) return;
                  e.preventDefault();
                  e.dataTransfer.dropEffect = "move";
                  setDragOver(space.id);
                }}
                onDragLeave={() => setDragOver((d) => (d === space.id ? null : d))}
                onDrop={(e) => {
                  e.preventDefault();
                  const id = dragId.current;
                  dragId.current = null;
                  setDragOver(null);
                  if (id && id !== space.id) move(id, i);
                }}
                onDragEnd={() => {
                  dragId.current = null;
                  setDragOver(null);
                }}
              >
                <button
                  type="button"
                  className="space-button"
                  aria-current={isSelected(space.id) ? "true" : undefined}
                  aria-keyshortcuts="Alt+ArrowUp Alt+ArrowDown"
                  onClick={() => onSelect({ module: "agents", id: space.id })}
                  onKeyDown={(e) => onKeyDown(e, i)}
                >
                  {space.label}
                </button>
                <ul className="plain agents">
                  {space.agents.map((a) => (
                    <li key={a.paneId}>
                      <button
                        type="button"
                        className="row"
                        aria-current={isSelected(a.paneId) ? "true" : undefined}
                        onClick={() => selectPane(a.paneId)}
                      >
                        <span className="row-title">{agentName(a)}</span>
                        <StateDot status={a.status} />
                      </button>
                    </li>
                  ))}
                </ul>
              </li>
            ))}
          </ul>
        </div>
      )}

      {state.connection === "connected" && state.spaces.length === 0 && <p className="muted">No spaces yet.</p>}

      <div role="status" className="sr-only">
        {announcement}
      </div>
    </div>
  );
}

function NeedsYouRow({ item, selected, onPick }: { item: NeedsYouItem; selected: boolean; onPick(id: string): void }) {
  return (
    <li>
      <button
        type="button"
        className="row"
        aria-current={selected ? "true" : undefined}
        onClick={() => onPick(item.paneId)}
      >
        <span className="row-title">{agentName(item)}</span>
        <span className="row-sub">{item.spaceLabel}</span>
        <StateDot status={item.status} />
      </button>
    </li>
  );
}
