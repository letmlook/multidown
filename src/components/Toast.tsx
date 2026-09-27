import { useEffect } from "react";

interface ToastProps {
  message: string;
  duration?: number;
  onClose: () => void;
}

export function Toast({ message, duration = 2000, onClose }: ToastProps) {
  useEffect(() => {
    const timer = setTimeout(onClose, duration);
    return () => clearTimeout(timer);
  }, [duration, onClose]);

  return (
    <div className="toast-overlay">
      <div className="toast">
        <div className="toast-message">{message}</div>
        <button className="toast-close" onClick={onClose}>
          确定
        </button>
      </div>
    </div>
  );
}
