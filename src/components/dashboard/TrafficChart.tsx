import { useId, useLayoutEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { useConnection, SAMPLE_COUNT } from "../../stores/connection";
import { useSettings } from "../../stores/settings";
import { formatBps } from "../../lib/format";
import { cn } from "../../lib/cn";

const TOP_PAD = 10;
/** samples visible at once; one extra sample slides in from the right */
const VISIBLE = SAMPLE_COUNT - 1;

type Pt = readonly [number, number];

/** Catmull-Rom spline converted to cubic beziers. */
function smoothPath(pts: Pt[]): string {
  if (pts.length < 2) return "";
  let d = `M ${pts[0][0].toFixed(2)} ${pts[0][1].toFixed(2)}`;
  for (let i = 0; i < pts.length - 1; i++) {
    const p0 = pts[Math.max(0, i - 1)];
    const p1 = pts[i];
    const p2 = pts[i + 1];
    const p3 = pts[Math.min(pts.length - 1, i + 2)];
    const c1x = p1[0] + (p2[0] - p0[0]) / 6;
    const c1y = p1[1] + (p2[1] - p0[1]) / 6;
    const c2x = p2[0] - (p3[0] - p1[0]) / 6;
    const c2y = p2[1] - (p3[1] - p1[1]) / 6;
    d += ` C ${c1x.toFixed(2)} ${c1y.toFixed(2)}, ${c2x.toFixed(2)} ${c2y.toFixed(2)}, ${p2[0].toFixed(2)} ${p2[1].toFixed(2)}`;
  }
  return d;
}

/**
 * A sine band spanning 2w with a period of exactly w, closed to the bottom.
 * Period === w is what makes a 100% horizontal translate seamless.
 */
function tidePath(w: number, h: number, baseline: number, amp: number, phase: number): string {
  if (w <= 0) return "";
  const steps = 48;
  const pts: string[] = [];
  for (let i = 0; i <= steps * 2; i++) {
    const x = (i / steps) * w;
    const y = baseline - Math.sin((i / steps) * Math.PI * 2 + phase) * amp;
    pts.push(`${i === 0 ? "M" : "L"} ${x.toFixed(2)} ${y.toFixed(2)}`);
  }
  return `${pts.join(" ")} L ${(2 * w).toFixed(2)} ${h} L 0 ${h} Z`;
}

export function TrafficChart() {
  const { t } = useTranslation();
  const samples = useConnection((s) => s.samples);
  const tick = useConnection((s) => s.tick);
  const status = useConnection((s) => s.conn.status);
  const reduceMotion = useSettings((s) => s.settings?.reduceMotion ?? false);
  const wrapRef = useRef<HTMLDivElement>(null);
  const gRef = useRef<SVGGElement>(null);
  const smoothMaxRef = useRef(1);
  const [size, setSize] = useState({ w: 0, h: 0 });
  const uid = useId().replace(/[^a-zA-Z0-9-]/g, "");

  useLayoutEffect(() => {
    const el = wrapRef.current;
    if (!el) return;
    const measure = () => setSize({ w: el.clientWidth, h: el.clientHeight });
    measure();
    const ro = new ResizeObserver(measure);
    ro.observe(el);
    return () => ro.disconnect();
  }, []);

  const { w, h } = size;
  const step = w > 0 ? w / (VISIBLE - 1) : 0;

  /**
   * "Nothing is flowing" — the state that used to render as a flat line pinned
   * to the bottom of an otherwise empty box. Covers both disconnected and the
   * first seconds of a connection, before any sample has arrived.
   */
  const idle = useMemo(
    () => samples.every((s) => s.up === 0 && s.down === 0),
    [samples],
  );

  // Window max, smoothed: jumps up instantly, decays gently.
  const rawMax = useMemo(
    () => Math.max(1, ...samples.map((s) => Math.max(s.up, s.down))),
    [samples],
  );
  const yMax = useMemo(() => {
    const next = Math.max(rawMax, smoothMaxRef.current * 0.94, 1);
    smoothMaxRef.current = next;
    return next;
  }, [rawMax]);

  const { downLine, downArea, upLine, upArea } = useMemo(() => {
    const y = (v: number) => h - Math.sqrt(Math.min(1, v / yMax)) * (h - TOP_PAD);
    const downPts: Pt[] = samples.map((s, i) => [i * step, y(s.down)] as const);
    const upPts: Pt[] = samples.map((s, i) => [i * step, y(s.up)] as const);
    const lx = (samples.length - 1) * step;
    const dl = smoothPath(downPts);
    const ul = smoothPath(upPts);
    return {
      downLine: dl,
      downArea: dl ? `${dl} L ${lx.toFixed(2)} ${h} L 0 ${h} Z` : "",
      upLine: ul,
      upArea: ul ? `${ul} L ${lx.toFixed(2)} ${h} L 0 ${h} Z` : "",
    };
  }, [samples, step, yMax, h]);

  const tides = useMemo(() => {
    const base = h * 0.72;
    return {
      a: tidePath(w, h, base, h * 0.075, 0),
      b: tidePath(w, h, base + h * 0.08, h * 0.055, Math.PI * 0.6),
    };
  }, [w, h]);

  // Conveyor: reset to 0 instantly on each new sample, then slide one step
  // left over 1s (linear) so the newest point glides in from the right.
  useLayoutEffect(() => {
    const g = gRef.current;
    if (!g || step === 0) return;
    g.style.transition = "none";
    g.style.transform = "translateX(0px)";
    void g.getBoundingClientRect();
    g.style.transition = "transform 1s linear";
    g.style.transform = `translateX(${-step}px)`;
  }, [tick, step]);

  return (
    <div
      ref={wrapRef}
      className="dashboard-chart relative h-[clamp(6.5rem,14vh,10rem)] w-full overflow-hidden rounded-(--radius-ctl)"
    >
      {w > 0 && h > 0 && (
        <svg
          width={w}
          height={h}
          viewBox={`0 0 ${w} ${h}`}
          className="block"
          aria-hidden
        >
          <defs>
            <linearGradient id={`${uid}-down`} x1="0" y1="0" x2="0" y2="1">
              <stop offset="0" stopColor="var(--color-accent-2)" stopOpacity="0.34" />
              <stop offset="1" stopColor="var(--color-accent-2)" stopOpacity="0" />
            </linearGradient>
            <linearGradient id={`${uid}-up`} x1="0" y1="0" x2="0" y2="1">
              <stop offset="0" stopColor="var(--color-accent)" stopOpacity="0.22" />
              <stop offset="1" stopColor="var(--color-accent)" stopOpacity="0" />
            </linearGradient>
            <linearGradient id={`${uid}-tide`} x1="0" y1="0" x2="0" y2="1">
              <stop offset="0" stopColor="var(--color-accent)" stopOpacity="0.2" />
              <stop offset="1" stopColor="var(--color-accent)" stopOpacity="0.02" />
            </linearGradient>
            <filter id={`${uid}-glow`} x="-20%" y="-40%" width="140%" height="180%">
              <feGaussianBlur stdDeviation="4" />
            </filter>
          </defs>

          {/* horizon gridlines — always on, so the box reads as a chart even empty */}
          {[0.28, 0.52, 0.76].map((f) => (
            <line
              key={f}
              x1="0"
              x2={w}
              y1={h * f}
              y2={h * f}
              stroke="var(--color-glass-border)"
              strokeWidth="1"
            />
          ))}

          {/*
            Resting tide. Present whenever no traffic has been sampled, so an
            idle dashboard shows a calm sea instead of dead space; it crossfades
            out the moment real data starts riding on top of it.
          */}
          <g
            style={{
              opacity: idle ? 1 : 0,
              transition: "opacity 700ms ease",
            }}
          >
            <path
              d={tides.b}
              fill={`url(#${uid}-tide)`}
              opacity="0.55"
              className={cn(!reduceMotion && "tide-b")}
            />
            <path
              d={tides.a}
              fill={`url(#${uid}-tide)`}
              className={cn(!reduceMotion && "tide-a")}
            />
          </g>

          <g
            ref={gRef}
            style={{
              willChange: "transform",
              opacity: idle ? 0 : 1,
              transition: "opacity 500ms ease",
            }}
          >
            {/* download: accent-2 area + neon stroke */}
            <path d={downArea} fill={`url(#${uid}-down)`} />
            <path
              d={downLine}
              fill="none"
              stroke="var(--color-accent-2)"
              strokeWidth="3"
              strokeOpacity="0.55"
              filter={`url(#${uid}-glow)`}
            />
            <path
              d={downLine}
              fill="none"
              stroke="var(--color-accent-2)"
              strokeWidth="2"
              strokeLinejoin="round"
            />
            {/* upload: accent overlay */}
            <path d={upArea} fill={`url(#${uid}-up)`} opacity="0.6" />
            <path
              d={upLine}
              fill="none"
              stroke="var(--color-accent)"
              strokeWidth="2"
              strokeOpacity="0.7"
              strokeLinejoin="round"
            />
          </g>
        </svg>
      )}

      {/* caption for the resting state */}
      <div
        className={cn(
          "pointer-events-none absolute inset-x-0 top-[26%] text-center",
          "text-label font-semibold tracking-[0.14em] text-text-faint uppercase",
          "transition-opacity duration-500",
          idle ? "opacity-100" : "opacity-0",
        )}
      >
        {status === "connected"
          ? t("dashboard.chart.waiting")
          : t("dashboard.chart.resting")}
      </div>

      {/* live window peak — density the empty box never carried */}
      <div
        className={cn(
          "pointer-events-none absolute top-1.5 right-2 font-mono text-[11px] tabular-nums",
          "text-text-faint transition-opacity duration-500",
          idle ? "opacity-0" : "opacity-100",
        )}
      >
        {t("dashboard.chart.peak", { value: formatBps(yMax) })}
      </div>
    </div>
  );
}
