import { useEffect, useMemo, useRef, useState } from 'react';
import { cn } from '../lib/utils';
import { getAccent } from '../lib/theme';
import { useTintedLogo } from '../lib/logoTint';

/* ── 版本号数字滚动：每个数字是竖排 0-9×2 胶片列，hover 滚入（级联延迟 + 回弹）── */

const DIGIT_CYCLE = '01234567890123456789';

type RollPart =
  | { kind: 'static'; char: string }
  | { kind: 'digit'; target: number; delay: number };

export function RollingText({ text, active }: { text: string; active: boolean }) {
  const parts = useMemo<RollPart[]>(
    () => {
      const out: RollPart[] = [];
      let delay = 0;
      for (const char of text) {
        if (char >= '0' && char <= '9') {
          out.push({ kind: 'digit', target: Number(char), delay });
          delay += 55;
        } else {
          out.push({ kind: 'static', char });
        }
      }
      return out;
    },
    [text],
  );

  const [index, setIndex] = useState<number[]>(() => parts.map((p) => (p.kind === 'digit' ? p.target : 0)));
  const [animating, setAnimating] = useState(false);

  useEffect(() => {
    let raf2 = 0;
    if (active) {
      // 先无动画跳回 0，两帧后带动画滚到目标数字
      setAnimating(false);
      setIndex(parts.map((p) => (p.kind === 'digit' ? 0 : 0)));
      raf2 = requestAnimationFrame(() =>
        requestAnimationFrame(() => {
          setAnimating(true);
          setIndex(parts.map((p) => (p.kind === 'digit' ? p.target : 0)));
        }),
      );
    } else {
      // 滚出：多转一圈（target+10）
      setAnimating(true);
      setIndex(parts.map((p) => (p.kind === 'digit' ? p.target + 10 : 0)));
    }
    return () => cancelAnimationFrame(raf2);
  }, [active, parts]);

  return (
    <span className="inline-flex items-baseline font-medium tabular-nums" aria-label={text}>
      {parts.map((p, i) =>
        p.kind === 'static' ? (
          <span key={i}>{p.char}</span>
        ) : (
          <span key={i} className="inline-block h-[1em] overflow-hidden align-baseline">
            <span
              className={cn(
                'flex flex-col will-change-transform',
                animating && 'transition-[transform] duration-[680ms] [transition-timing-function:cubic-bezier(0.22,1.12,0.36,1)]',
              )}
              style={{
                transform: `translateY(-${index[i] ?? 0}em)`,
                transitionDelay: animating ? `${p.delay}ms` : undefined,
              }}
            >
              {DIGIT_CYCLE.split('').map((d, di) => (
                <span key={di} className="flex h-[1em] items-center justify-center leading-none">
                  {d}
                </span>
              ))}
            </span>
          </span>
        ),
      )}
    </span>
  );
}

/* ── Hero 背景动效：光晕漂移（CSS）+ 丝带线条（SVG 漂移/呼吸）+ 粒子（rAF）── */

interface FxParticle {
  x: number; y: number; vx: number; vy: number;
  depth: number; band: 'far' | 'mid' | 'near';
  phase: number; seed: number; wander: number; burstIn: number;
}

/** 丝带：三景深平面，每条一个手写贝塞尔路径（viewBox 0 0 400 80） */
interface FxRibbon {
  d: string;
  depth: 'far' | 'mid' | 'near';
  seed: number;
  blur: 'glow' | 'soft' | 'mid' | 'heavy';
}

const FX_RIBBONS: FxRibbon[] = [
  { depth: 'far', seed: 11.3, blur: 'glow', d: 'M-52,44 C55,30 155,54 245,36 S395,50 452,34' },
  { depth: 'far', seed: 27.8, blur: 'soft', d: 'M-52,60 C80,72 185,44 278,62 S388,40 452,56' },
  { depth: 'far', seed: 43.1, blur: 'mid', d: 'M-52,28 C105,38 210,18 305,32 S380,24 452,40' },
  { depth: 'mid', seed: 58.6, blur: 'glow', d: 'M-52,34 C90,50 205,22 302,42 S382,28 452,46' },
  { depth: 'mid', seed: 71.2, blur: 'soft', d: 'M-52,52 C60,38 172,64 262,44 S372,58 452,36' },
  { depth: 'mid', seed: 89.5, blur: 'mid', d: 'M-52,40 C73,56 198,28 292,52 S396,34 452,48' },
  { depth: 'near', seed: 104.7, blur: 'heavy', d: 'M-52,46 C100,58 218,32 318,54 S390,38 452,50' },
  { depth: 'near', seed: 118.9, blur: 'soft', d: 'M-52,36 C77,24 188,58 284,38 S398,52 452,42' },
];

const seeded01 = (seed: number) => {
  const x = Math.sin(seed * 127.1) * 43758.5453;
  return x - Math.floor(x);
};
const clamp = (v: number, lo: number, hi: number) => Math.max(lo, Math.min(hi, v));
const maxSpeed = (d: number) => 0.014 + d * 0.038;
const ANIM_SCALE = 0.38;
const depthMul = (d: string) => (d === 'near' ? 1.4 : d === 'mid' ? 1 : 0.72);

/** 丝带整体漂移（g transform） */
function ribbonWrapStyle(r: FxRibbon, t: number): string {
  const time = t * ANIM_SCALE;
  const mul = depthMul(r.depth);
  const s = r.seed;
  const x = (Math.sin(time * 0.62 + s) * 16 + Math.sin(time * 1.05 + s * 1.6) * 9) * mul;
  const y = (Math.cos(time * 0.54 + s * 1.2) * 12 + Math.cos(time * 0.88 + s * 0.7) * 7) * mul;
  const rot = Math.sin(time * 0.36 + s * 0.5) * 2.2 * mul;
  const scale = 1 + Math.sin(time * 0.44 + s * 0.85) * 0.04 * mul;
  return `translate(${x.toFixed(2)}px,${y.toFixed(2)}px) rotate(${rot.toFixed(2)}deg) scale(${scale.toFixed(4)})`;
}

/** 丝带对焦呼吸（path opacity + blur/drop-shadow） */
function ribbonPathStyle(r: FxRibbon, t: number): { opacity: number; filter: string } {
  const time = t * ANIM_SCALE;
  const focus = 0.5 + 0.5 * Math.sin(time * 0.72 + r.seed * 1.15);
  const base = r.blur === 'glow' ? 0.48 : r.blur === 'soft' ? 0.4 : r.blur === 'mid' ? 0.34 : 0.26;
  const swing = r.blur === 'glow' ? 0.14 : 0.15;
  let filter: string;
  switch (r.blur) {
    case 'glow':
      filter = `blur(${(0.35 + focus * 0.35).toFixed(2)}px) drop-shadow(0 0 ${(2.5 + focus * 3).toFixed(1)}px color-mix(in srgb, var(--brand) 30%, transparent))`;
      break;
    case 'soft':
      filter = `blur(${(1.5 + (1 - focus) * 1.1).toFixed(2)}px)`;
      break;
    case 'mid':
      filter = `blur(${(3.8 + (1 - focus) * 2.2).toFixed(2)}px)`;
      break;
    default:
      filter = `blur(${(11.5 + (1 - focus) * 3.5).toFixed(2)}px)`;
  }
  return { opacity: base + focus * swing, filter };
}

function pickVelocity(p: FxParticle, scale = 1) {
  const angle = p.wander + (Math.random() - 0.5) * Math.PI * 1.35;
  const spd = maxSpeed(p.depth) * scale * (0.35 + Math.random() * 0.75);
  p.vx = Math.cos(angle) * spd;
  p.vy = Math.sin(angle) * spd;
}

function createParticles(): FxParticle[] {
  return Array.from({ length: 9 }, (_, i) => {
    const s1 = seeded01(i * 3.71 + 1.2);
    const s2 = seeded01(i * 5.13 + 2.8);
    const s3 = seeded01(i * 7.91 + 0.4);
    const depth = 0.1 + i * 0.09 + (s3 - 0.5) * 0.05;
    const p: FxParticle = {
      x: 0.04 + s1 * 0.92,
      y: 0.08 + s2 * 0.84,
      vx: 0, vy: 0,
      depth,
      band: depth < 0.35 ? 'far' : depth < 0.68 ? 'mid' : 'near',
      phase: s2 * Math.PI * 2,
      seed: i * 13.7 + s3 * 100,
      wander: s1 * Math.PI * 2,
      burstIn: seeded01(i * 11.3) * 4,
    };
    pickVelocity(p, 0.85 + s3 * 0.5);
    return p;
  });
}

function particleStyle(p: FxParticle): React.CSSProperties {
  const pulseSpeed = 0.75 + p.depth * 0.95 + seeded01(p.seed * 9.4) * 0.5;
  const pulse = 0.5 + 0.5 * Math.sin(p.phase * pulseSpeed + p.seed * 0.08);
  const wobble2 = Math.sin(p.phase * (3.2 + seeded01(p.seed * 11.7) * 2.4) + p.seed * 1.9);
  const wobble3 = Math.cos(p.phase * (1.5 + seeded01(p.seed * 14.3) * 1.6) + p.seed * 2.3);
  const baseSize = p.band === 'near' ? 16 + p.depth * 12 : p.band === 'mid' ? 6 + p.depth * 5 : 3 + p.depth * 2.5;
  const size = baseSize * (0.88 + pulse * 0.22 * (0.4 + p.depth * 0.6));
  const baseBlur = p.band === 'near' ? 12 + p.depth * 10 : p.band === 'mid' ? 1.8 + p.depth * 3 : 0.5 + p.depth;
  const blur = baseBlur * (1 + (1 - pulse) * 0.55);
  const baseOpacity = p.band === 'near' ? 0.22 : p.band === 'mid' ? 0.32 + p.depth * 0.14 : 0.42 + p.depth * 0.18;
  const px = (pulse - 0.5) * (10 + p.depth * 22) + wobble2 * (4 + p.depth * 8);
  const py = (pulse - 0.5) * (8 + p.depth * 14) + wobble3 * (3 + p.depth * 7);
  return {
    left: `calc(${(p.x * 100).toFixed(2)}% + ${px.toFixed(1)}px)`,
    top: `calc(${(p.y * 100).toFixed(2)}% + ${py.toFixed(1)}px)`,
    width: `${size.toFixed(1)}px`,
    height: `${size.toFixed(1)}px`,
    opacity: Number((baseOpacity * (0.75 + pulse * 0.35)).toFixed(3)),
    transform: `translate(-50%,-50%) scale(${(0.84 + pulse * 0.24).toFixed(3)})`,
    filter: `blur(${blur.toFixed(1)}px)`,
  };
}

function updateParticles(ps: FxParticle[], dt: number, t: number) {
  const step = Math.min(dt, 0.05);
  for (const p of ps) {
    p.phase += step * (0.38 + p.depth * 0.32);
    p.burstIn -= step;
    const ds = 0.5 + p.depth * 0.5;
    p.vx += Math.sin(p.phase * 2.3 + p.seed) * 0.007 * ds + Math.sin(t * 0.45 + p.seed) * 0.0022;
    p.vy += Math.cos(p.phase * 1.65 + p.seed * 1.4) * 0.007 * ds + Math.cos(t * 0.36 + p.seed * 0.7) * 0.0018;
    p.wander += (Math.random() - 0.5) * (0.028 + p.depth * 0.04) * step;
    const pull = 0.0022 * (0.45 + p.depth * 0.55);
    p.vx += Math.cos(p.wander) * pull;
    p.vy += Math.sin(p.wander) * pull;
    if (Math.random() < 0.012 * ds && p.burstIn <= 0) {
      pickVelocity(p, 0.75 + Math.random() * 0.9);
      p.burstIn = 1.2 + Math.random() * 2.8;
    }
    const cap = maxSpeed(p.depth);
    const spd = Math.hypot(p.vx, p.vy);
    if (spd > cap) {
      p.vx = (p.vx / spd) * cap;
      p.vy = (p.vy / spd) * cap;
    }
    p.x += p.vx * step;
    p.y += p.vy * step;
    if (p.x < 0.02 || p.x > 0.98 || p.y < 0.06 || p.y > 0.94) {
      p.x = clamp(p.x, 0.04, 0.96);
      p.y = clamp(p.y, 0.08, 0.92);
      p.wander = Math.atan2(0.5 - p.y, 0.5 - p.x) + (Math.random() - 0.5) * 2.4;
      pickVelocity(p, 0.5);
    }
  }
}

/** Hero 横幅（参考 Wanwu 关于页）：logo + 名称 + 版本号（hover 数字滚动）+ 粒子背景。 */
export function AboutHero({ name, version }: { name: string; version: string }) {
  const [hover, setHover] = useState(false);
  const fxRef = useRef<HTMLDivElement>(null);
  const particles = useRef<FxParticle[]>(createParticles());

  // logo 直接采用主题色 H+S 着色（与托盘/侧栏同源）；主题色切换即时刷新
  const [accentHue, setAccentHue] = useState(() => getAccent().hue);
  const [accentSat, setAccentSat] = useState(() => getAccent().sat);
  useEffect(() => {
    const on = (e: Event) => {
      const a = (e as CustomEvent<{ hue: number; sat: number }>).detail;
      setAccentHue(a.hue);
      setAccentSat(a.sat);
    };
    window.addEventListener('main:accent', on);
    return () => window.removeEventListener('main:accent', on);
  }, []);
  const logoSrc = useTintedLogo(accentHue, accentSat);

  useEffect(() => {
    if (!hover) return;
    const spans = Array.from(fxRef.current?.querySelectorAll<HTMLElement>('[data-particle]') ?? []);
    const ribbonGs = Array.from(fxRef.current?.querySelectorAll<SVGGElement>('[data-ribbon-wrap]') ?? []);
    const ribbonPaths = Array.from(fxRef.current?.querySelectorAll<SVGPathElement>('[data-ribbon]') ?? []);
    let raf = 0;
    let last = 0;
    let t = 0;
    const tick = (now: number) => {
      if (!last) last = now;
      const dt = (now - last) / 1000;
      last = now;
      t += dt;
      updateParticles(particles.current, dt, t);
      spans.forEach((el, i) => Object.assign(el.style, particleStyle(particles.current[i])));
      ribbonGs.forEach((g, i) => {
        g.style.transform = ribbonWrapStyle(FX_RIBBONS[i], t);
      });
      ribbonPaths.forEach((p, i) => {
        const s = ribbonPathStyle(FX_RIBBONS[i], t);
        p.style.opacity = String(s.opacity);
        p.style.filter = s.filter;
      });
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [hover]);

  return (
    <div
      className="group relative mb-4 min-h-[5.5rem] overflow-hidden rounded-2xl border border-[var(--glass-border)] bg-[var(--group-bg)] transition-all duration-300 hover:-translate-y-px hover:shadow-[var(--shadow-lg)] [isolation:isolate]"
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
    >
      {/* 背景动效层 */}
      <div
        ref={fxRef}
        className="pointer-events-none absolute inset-0 z-0 transition-opacity duration-500"
        style={{ opacity: hover ? 1 : 0 }}
        aria-hidden
      >
        {/* 光晕（CSS 漂移） */}
        <span className="absolute left-[6%] top-[-14%] size-3/5 rounded-full bg-[radial-gradient(circle,color-mix(in_srgb,var(--brand)_18%,transparent),transparent_68%)] blur-2xl [animation:hero-halo-a_36s_ease-in-out_infinite]" />
        <span className="absolute right-[4%] top-[-4%] size-1/2 rounded-full bg-[radial-gradient(circle,color-mix(in_srgb,var(--brand)_14%,transparent),transparent_68%)] blur-2xl [animation:hero-halo-b_30s_ease-in-out_infinite]" />
        {/* 丝带线条（三景深平面，rAF 漂移/呼吸） */}
        {(['far', 'mid', 'near'] as const).map((depth) => (
          <svg
            key={depth}
            className="absolute inset-0 h-full w-full overflow-visible"
            viewBox="0 0 400 80"
            preserveAspectRatio="none"
          >
            {FX_RIBBONS.map((r, i) =>
              r.depth === depth ? (
                <g
                  key={i}
                  data-ribbon-wrap
                  style={{ transformBox: 'fill-box', transformOrigin: 'center' }}
                >
                  <path
                    data-ribbon
                    d={r.d}
                    fill="none"
                    strokeLinecap="round"
                    strokeLinejoin="round"
                    vectorEffect="non-scaling-stroke"
                    stroke={`color-mix(in srgb, var(--brand) ${depth === 'near' ? 24 : depth === 'mid' ? 34 : 26}%, transparent)`}
                    strokeWidth={depth === 'near' ? 2.8 : depth === 'mid' ? 1.55 : 1.1}
                  />
                </g>
              ) : null,
            )}
          </svg>
        ))}
        {/* 粒子（rAF 驱动，景深模糊） */}
        {particles.current.map((p, i) => (
          <span
            key={i}
            data-particle
            className="absolute rounded-full bg-[color-mix(in_srgb,var(--brand)_55%,transparent)]"
            style={particleStyle(p)}
          />
        ))}
        {/* 雾 + 暗角 */}
        <span className="absolute inset-x-0 top-0 h-2/5 bg-gradient-to-b from-[color-mix(in_srgb,var(--brand)_6%,transparent)] to-transparent" />
        <span className="absolute inset-0 bg-[radial-gradient(ellipse_85%_70%_at_50%_50%,transparent_42%,rgb(0_0_0/0.04))]" />
      </div>

      {/* 内容层 */}
      <div className="relative z-[2] flex items-center gap-4 px-6 py-5">
        <img
          src={logoSrc}
          alt={name}
          className="size-14 shrink-0 rounded-[0.875rem] object-cover transition-transform duration-300 group-hover:scale-105 group-hover:-rotate-2"
        />
        <div className="min-w-0 flex-1 transition-transform duration-200 group-hover:translate-x-0.5">
          <h2 className="text-xl font-semibold tracking-tight text-[var(--text-primary)]">{name}</h2>
          <p className="mt-1 text-label text-[var(--text-secondary)]">
            © {new Date().getFullYear()} Mono Studio
          </p>
        </div>
        <span className="shrink-0 text-lg font-semibold text-[var(--text-secondary)] transition-colors group-hover:text-[var(--text-primary)]">
          <span className="mr-1">v</span>
          <RollingText text={version} active={hover} />
        </span>
      </div>
    </div>
  );
}
