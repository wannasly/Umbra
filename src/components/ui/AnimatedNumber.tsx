import { useEffect, useRef, useState } from "react";
import { cn } from "../../lib/cn";

interface AnimatedNumberProps {
  value: number;
  format: (v: number) => string;
  className?: string;
}

/** Tweens between values over 300ms (ease-out cubic) and renders formatted. */
export function AnimatedNumber({ value, format, className }: AnimatedNumberProps) {
  const [display, setDisplay] = useState(value);
  const currentRef = useRef(value);
  const rafRef = useRef(0);

  useEffect(() => {
    const from = currentRef.current;
    const to = value;
    if (from === to) return;
    const start = performance.now();
    const duration = 300;
    const step = (now: number) => {
      const p = Math.min(1, (now - start) / duration);
      const eased = 1 - (1 - p) ** 3;
      const v = from + (to - from) * eased;
      currentRef.current = v;
      setDisplay(v);
      if (p < 1) rafRef.current = requestAnimationFrame(step);
    };
    rafRef.current = requestAnimationFrame(step);
    return () => cancelAnimationFrame(rafRef.current);
  }, [value]);

  return <span className={cn("tabular-nums", className)}>{format(display)}</span>;
}
