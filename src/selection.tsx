import React, { useEffect, useMemo, useState } from "react";
import { createRoot } from "react-dom/client";
import { listen } from "@tauri-apps/api/event";
import { Check, X } from "lucide-react";
import type { SelectionRect } from "./types";
import { invokeCommand } from "./tauri";
import "./selection.css";

interface Point {
  x: number;
  y: number;
}

interface DragRect {
  left: number;
  top: number;
  width: number;
  height: number;
}

function toRect(start: Point, end: Point): DragRect {
  const left = Math.min(start.x, end.x);
  const top = Math.min(start.y, end.y);
  return {
    left,
    top,
    width: Math.abs(end.x - start.x),
    height: Math.abs(end.y - start.y)
  };
}

function SelectionOverlay() {
  const [start, setStart] = useState<Point | null>(null);
  const [current, setCurrent] = useState<Point | null>(null);
  const [locked, setLocked] = useState<DragRect | null>(null);
  const [confirmed, setConfirmed] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const rect = useMemo(() => {
    if (locked) return locked;
    if (!start || !current) return null;
    return toRect(start, current);
  }, [current, locked, start]);

  useEffect(() => {
    const handleKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") void cancel();
      if (event.key === "Enter") void confirm();
    };

    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  });

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    listen("selection-reset", () => {
      setStart(null);
      setCurrent(null);
      setLocked(null);
      setConfirmed(false);
      setError(null);
    })
      .then((stop) => {
        unlisten = stop;
      })
      .catch((err) => setError(String(err)));

    return () => unlisten?.();
  }, []);

  function handlePointerDown(event: React.PointerEvent<HTMLDivElement>) {
    if (confirmed) return;
    if ((event.target as HTMLElement).closest(".selection-toolbar")) return;
    const point = { x: event.clientX, y: event.clientY };
    setStart(point);
    setCurrent(point);
    setLocked(null);
    setError(null);
    event.currentTarget.setPointerCapture(event.pointerId);
  }

  function handlePointerMove(event: React.PointerEvent<HTMLDivElement>) {
    if (!start || locked || confirmed) return;
    setCurrent({ x: event.clientX, y: event.clientY });
  }

  function handlePointerUp(event: React.PointerEvent<HTMLDivElement>) {
    if (!start || !current || confirmed) return;
    const next = toRect(start, { x: event.clientX, y: event.clientY });
    setLocked(next);
    setStart(null);
    setCurrent(null);
  }

  async function confirm() {
    if (!rect || rect.width < 24 || rect.height < 24) {
      setError("选区过小");
      return;
    }

    const selection: SelectionRect = {
      monitorId: "primary",
      x: Math.round(rect.left),
      y: Math.round(rect.top),
      width: Math.round(rect.width),
      height: Math.round(rect.height),
      scaleFactor: window.devicePixelRatio || 1
    };

    try {
      await invokeCommand("confirm_selection", { selection });
      setConfirmed(true);
    } catch (err) {
      setError(String(err));
    }
  }

  async function cancel() {
    try {
      await invokeCommand("cancel_selection");
    } catch (err) {
      setError(String(err));
    }
  }

  const toolbarStyle =
    rect && rect.width > 0 && rect.height > 0
      ? {
          left: Math.min(rect.left + rect.width - 148, window.innerWidth - 164),
          top: Math.min(rect.top + rect.height + 12, window.innerHeight - 58)
        }
      : undefined;

  return (
    <div
      className="selection-overlay"
      data-confirmed={confirmed}
      onPointerDown={handlePointerDown}
      onPointerMove={handlePointerMove}
      onPointerUp={handlePointerUp}
    >
      {!confirmed ? <div className="selection-copy">拖拽框选作文区域</div> : null}
      {rect && rect.width > 0 && rect.height > 0 ? (
        <>
          <div
            className="selection-rect"
            style={{
              left: rect.left,
              top: rect.top,
              width: rect.width,
              height: rect.height
            }}
          />
          {!confirmed ? (
            <div className="selection-toolbar" style={toolbarStyle}>
              <button onClick={confirm} title="确认选区">
                <Check size={18} />
              </button>
              <button onClick={cancel} title="取消">
                <X size={18} />
              </button>
            </div>
          ) : null}
        </>
      ) : null}
      {error ? <div className="selection-error">{error}</div> : null}
    </div>
  );
}

createRoot(document.getElementById("selection-root")!).render(<SelectionOverlay />);
