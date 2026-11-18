import { useEffect, useState } from "react";
import { onSelectAgent } from "./core/ipc";
import { modules, type Selection } from "./core/modules";

export default function App() {
  const [selection, setSelection] = useState<Selection>(null);

  useEffect(() => {
    let live = true;
    let unlisten: (() => void) | undefined;
    onSelectAgent((id) => setSelection({ module: "agents", id })).then((u) => (live ? (unlisten = u) : u()));
    return () => {
      live = false;
      unlisten?.();
    };
  }, []);

  const active = modules.find((m) => m.id === (selection?.module ?? modules[0]?.id));
  return (
    <>
      <nav aria-label="Agents">
        {modules.map((m) => m.Sidebar && <m.Sidebar key={m.id} selection={selection} onSelect={setSelection} />)}
      </nav>
      <main>{active?.Main && <active.Main selection={selection} />}</main>
    </>
  );
}
