import { useCallback, useState } from "react";

export function useToast() {
  const [toast, setToast] = useState<{ message: string; id: number } | null>(null);

  const showToast = useCallback((message: string) => {
    setToast((current) => ({ message, id: (current?.id ?? 0) + 1 }));
  }, []);

  const hideToast = useCallback(() => {
    setToast(null);
  }, []);

  return { toast, showToast, hideToast };
}
