import { useEffect } from "react";

export interface ContextMenuItem {
  label: string;
  onClick: () => void;
  disabled?: boolean;
}

interface ContextMenuProps {
  x: number;
  y: number;
  items: ContextMenuItem[];
  onClose: () => void;
}

export function ContextMenu({ x, y, items, onClose }: ContextMenuProps) {
  useEffect(() => {
    const close = () => onClose();
    document.addEventListener("click", close);
    document.addEventListener("contextmenu", close);
    return () => {
      document.removeEventListener("click", close);
      document.removeEventListener("contextmenu", close);
    };
  }, [onClose]);

  return (
    <div
      className="context-menu"
      style={{ left: x, top: y }}
      onClick={(e) => e.stopPropagation()}
    >
      {items.map((item) => (
        <div
          key={item.label}
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
      ))}
    </div>
  );
}
