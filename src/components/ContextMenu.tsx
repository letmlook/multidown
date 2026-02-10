import { useEffect, useLayoutEffect, useRef, useState } from "react";

export type ContextMenuItem =
  | { type: "item"; label: string; onClick: () => void; disabled?: boolean }
  | { type: "separator" }
  | { type: "submenu"; label: string; children: ContextMenuItem[] };

interface ContextMenuProps {
  x: number;
  y: number;
  items: ContextMenuItem[];
  onClose: () => void;
}

function renderItem(item: ContextMenuItem, key: string | number, onClose: () => void) {
  if (item.type === "separator") {
    return <div key={key} className="context-menu-separator" />;
  }
  if (item.type === "submenu") {
    return <ContextSubmenu key={key} label={item.label} children={item.children} onClose={onClose} />;
  }
  return (
    <div
      key={key}
      className={`context-menu-item ${item.disabled ? "context-menu-item-disabled" : ""}`}
      onClick={() => {
        if (!item.disabled) {
          item.onClick();
          onClose();
        }
      }}
    >
      {item.label}
    </div>
  );
}

function ContextSubmenu({
  label,
  children,
  onClose,
}: {
  label: string;
  children: ContextMenuItem[];
  onClose: () => void;
}) {
  const [open, setOpen] = useState(false);
  return (
    <div
      className="context-menu-submenu-wrap"
      onMouseEnter={() => setOpen(true)}
      onMouseLeave={() => setOpen(false)}
    >
      <div className="context-menu-item context-menu-item-has-sub">
        {label}
        <span className="context-menu-arrow">▶</span>
      </div>
      {open && (
        <div className="context-menu context-menu-sub" onClick={(e) => e.stopPropagation()}>
          {children.map((c, i) =>
            c.type === "separator" ? renderItem(c, `s-${i}`, onClose) : renderItem(c, `c-${i}`, onClose)
          )}
        </div>
      )}
    </div>
  );
}

export function ContextMenu({ x, y, items, onClose }: ContextMenuProps) {
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ left: x, top: y });

  useLayoutEffect(() => {
    const el = ref.current;
    if (!el) return;
    const rect = el.getBoundingClientRect();
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    let left = x;
    let top = y;
    if (rect.right > vw) left = Math.max(0, vw - rect.width);
    if (rect.bottom > vh) top = Math.max(0, vh - rect.height);
    if (rect.left < 0) left = 0;
    if (rect.top < 0) top = 0;
    setPos({ left, top });
  }, [x, y]);

  useEffect(() => {
    const close = () => onClose();
    const timer = setTimeout(() => {
      document.addEventListener("click", close);
      document.addEventListener("contextmenu", close);
    }, 0);
    return () => {
      clearTimeout(timer);
      document.removeEventListener("click", close);
      document.removeEventListener("contextmenu", close);
    };
  }, [onClose]);

  return (
    <div
      ref={ref}
      className="context-menu"
      style={{ left: pos.left, top: pos.top }}
      onClick={(e) => e.stopPropagation()}
    >
      {items.map((item, i) => renderItem(item, i, onClose))}
    </div>
  );
}
