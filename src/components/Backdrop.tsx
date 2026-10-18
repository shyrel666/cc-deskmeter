/** Fixed page backdrop; CSS shows only the layer for the active theme. */
export function Backdrop() {
  return (
    <div className="pulse-backdrop" aria-hidden>
      <div className="bd-warm" />
      <div className="bd-swiss" />
      <div className="bd-glass">
        <div className="orb -left-[10%] -top-[18%] h-[60vh] w-[50vw] bg-indigo-500/15" />
        <div className="orb -bottom-[22%] -right-[8%] h-[62vh] w-[48vw] bg-rose-500/10 [animation-delay:-9s]" />
        <div className="orb left-[38%] top-[34%] h-[38vh] w-[30vw] bg-violet-500/[0.08] [animation-delay:-17s]" />
      </div>
    </div>
  );
}
