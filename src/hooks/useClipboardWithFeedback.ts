import { useState, useCallback, useRef, useEffect } from 'react';

/**
 * React hook that copies text to the system clipboard and provides visual feedback.
 * @param duration - Duration (ms) the `copied` flag stays true (default 2000).
 * @returns `{ copied, copy }` — `copied` is true while feedback is active; `copy(text)` triggers the clipboard write.
 */
export function useClipboardWithFeedback(duration = 2000) {
  const [copied, setCopied] = useState(false);
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    return () => { if (timerRef.current) clearTimeout(timerRef.current); };
  }, []);

  const copy = useCallback(
    (text: string) => {
      navigator.clipboard.writeText(text).then(() => {
        setCopied(true);
        if (timerRef.current) clearTimeout(timerRef.current);
        timerRef.current = setTimeout(() => setCopied(false), duration);
      }).catch(() => {});
    },
    [duration],
  );

  return { copied, copy };
}
