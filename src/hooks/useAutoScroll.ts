import { useEffect, useRef, useState } from 'react';

export function useAutoScroll<T>(trigger: T) {
  const [enabled, setEnabled] = useState(true);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (enabled && ref.current) {
      ref.current.scrollTop = ref.current.scrollHeight;
    }
  }, [trigger, enabled]);

  return { ref, enabled, setEnabled };
}
