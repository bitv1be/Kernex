import { cn } from "@/lib/utils";

export function ResizeHandle({ side, onResize }: { side: "left" | "right"; onResize: (delta: number) => void }) {
  const onPointerDown = (event: React.PointerEvent<HTMLDivElement>) => {
    const origin = event.clientX;
    const target = event.currentTarget;
    target.setPointerCapture(event.pointerId);
    const move = (moveEvent: PointerEvent) => onResize((moveEvent.clientX - origin) * (side === "right" ? 1 : -1));
    const stop = () => {
      target.removeEventListener("pointermove", move);
      target.removeEventListener("pointerup", stop);
    };
    target.addEventListener("pointermove", move);
    target.addEventListener("pointerup", stop);
  };
  return <div role="separator" aria-label="Resize panel" aria-orientation="vertical" tabIndex={0} className={cn("absolute inset-y-0 z-20 w-1 cursor-col-resize touch-none hover:bg-ring/40 focus:bg-ring/40", side === "right" ? "-right-0.5" : "-left-0.5")} onPointerDown={onPointerDown} onKeyDown={(event) => { if (event.key === "ArrowLeft") onResize(side === "right" ? -12 : 12); if (event.key === "ArrowRight") onResize(side === "right" ? 12 : -12); }} />;
}
