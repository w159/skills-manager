# CSS Patterns for UI Overhaul

Full CSS code for design system extensions. Add these to the project's global stylesheet (e.g. `src/index.css`), extending existing `@layer base`, `@layer components`, and `@layer utilities`.

## Extended Color Palette (Light Theme)

```css
:root {
  /* Extended palette — alongside existing accent/danger */
  --color-sky: #0EA5E9;
  --color-sky-light: #38BDF8;
  --color-sky-bg: rgba(14,165,233,0.08);
  --color-violet: #8B5CF6;
  --color-violet-light: #A78BFA;
  --color-violet-bg: rgba(139,92,246,0.08);
  --color-amber: #F59E0B;
  --color-amber-light: #FBBF24;
  --color-amber-bg: rgba(245,158,11,0.08);
  --color-rose: #F43F5E;
  --color-rose-light: #FB7185;
  --color-rose-bg: rgba(244,63,94,0.08);
  --color-blue: #3B82F6;
  --color-blue-light: #60A5FA;
  --color-blue-bg: rgba(59,130,246,0.08);

  /* Glass */
  --glass-blur: 12px;
  --glass-bg: rgba(255,255,255,0.72);
  --glass-border: rgba(255,255,255,0.18);

  /* Shadows */
  --shadow-sm: 0 1px 2px rgba(0,0,0,0.04);
  --shadow-md: 0 2px 8px rgba(0,0,0,0.06);
  --shadow-lg: 0 8px 24px rgba(0,0,0,0.08);
  --shadow-glow: 0 0 20px rgba(16,185,129,0.12);

  /* Animation easing */
  --ease-spring: cubic-bezier(0.34, 1.56, 0.64, 1);
  --ease-out: cubic-bezier(0.16, 1, 0.3, 1);
  --ease-in-out: cubic-bezier(0.65, 0, 0.35, 1);
}
```

## Dark Theme Overrides

```css
.dark {
  --color-sky: #38BDF8;
  --color-sky-light: #7DD3FC;
  --color-sky-bg: rgba(56,189,248,0.08);
  --color-violet: #A78BFA;
  --color-violet-light: #C4B5FD;
  --color-violet-bg: rgba(167,139,250,0.08);
  --color-amber: #FBBF24;
  --color-amber-light: #FCD34D;
  --color-amber-bg: rgba(251,191,36,0.08);
  --color-rose: #FB7185;
  --color-rose-light: #FDA4AF;
  --color-rose-bg: rgba(251,113,133,0.08);
  --color-blue: #60A5FA;
  --color-blue-light: #93C5FD;
  --color-blue-bg: rgba(96,165,250,0.08);

  --glass-blur: 16px;
  --glass-bg: rgba(19,19,23,0.72);
  --glass-border: rgba(255,255,255,0.06);

  --shadow-sm: 0 1px 2px rgba(0,0,0,0.2);
  --shadow-md: 0 2px 8px rgba(0,0,0,0.3);
  --shadow-lg: 0 8px 24px rgba(0,0,0,0.4);
  --shadow-glow: 0 0 20px rgba(52,211,153,0.12);
}
```

## Component Utility Classes

```css
@layer components {
  /* Skeleton shimmer */
  .skeleton {
    position: relative;
    overflow: hidden;
    background: var(--color-surface-hover);
    border-radius: 8px;
  }
  .skeleton::after {
    position: absolute;
    inset: 0;
    transform: translateX(-100%);
    background: linear-gradient(90deg, transparent, rgba(255,255,255,0.04), transparent);
    animation: shimmer 1.5s infinite;
    content: "";
  }

  /* Animated gradient text */
  .gradient-text {
    background: linear-gradient(135deg, var(--color-accent-light), var(--color-sky), var(--color-violet));
    background-size: 200% 200%;
    -webkit-background-clip: text;
    background-clip: text;
    -webkit-text-fill-color: transparent;
    animation: gradient-shift 6s ease-in-out infinite;
  }

  /* Glow ring */
  .glow-ring {
    box-shadow: 0 0 0 1px var(--color-accent-border), 0 0 16px var(--color-accent-bg);
  }

  /* Pulse dot */
  .pulse-dot { position: relative; }
  .pulse-dot::after {
    position: absolute;
    inset: -2px;
    border-radius: 9999px;
    background: currentColor;
    opacity: 0.3;
    animation: pulse-ring 2s ease-out infinite;
    content: "";
  }

  /* Entrance animations */
  .animate-fade-in { animation: fade-in 0.4s var(--ease-out) both; }
  .animate-fade-in-up { animation: fade-in-up 0.5s var(--ease-out) both; }
  .animate-scale-in { animation: scale-in 0.35s var(--ease-spring) both; }
  .animate-slide-in-right { animation: slide-in-right 0.3s var(--ease-out) both; }

  /* Staggered children */
  .stagger-children > * {
    opacity: 0;
    animation: stagger-fade-in 0.4s var(--ease-out) forwards;
  }
  .stagger-children > *:nth-child(1) { animation-delay: 0ms; }
  .stagger-children > *:nth-child(2) { animation-delay: 40ms; }
  .stagger-children > *:nth-child(3) { animation-delay: 80ms; }
  .stagger-children > *:nth-child(4) { animation-delay: 120ms; }
  .stagger-children > *:nth-child(5) { animation-delay: 160ms; }
  .stagger-children > *:nth-child(6) { animation-delay: 200ms; }
  .stagger-children > *:nth-child(7) { animation-delay: 240ms; }
  .stagger-children > *:nth-child(8) { animation-delay: 280ms; }
  .stagger-children > *:nth-child(9) { animation-delay: 320ms; }
  .stagger-children > *:nth-child(10) { animation-delay: 360ms; }

  /* Hover lift */
  .hover-lift {
    transition: transform 0.2s var(--ease-spring), box-shadow 0.2s var(--ease-out), border-color 0.2s var(--ease-out);
  }
  .hover-lift:hover { transform: translateY(-2px); box-shadow: var(--shadow-md); }

  /* Progress bar */
  .progress-bar { position: relative; overflow: hidden; background: var(--color-surface-hover); border-radius: 9999px; }
  .progress-bar-fill {
    height: 100%;
    border-radius: 9999px;
    background: linear-gradient(90deg, var(--color-accent), var(--color-accent-light));
    transition: width 0.5s var(--ease-out);
  }
  .progress-bar-fill.indeterminate { width: 40% !important; animation: indeterminate 1.4s ease-in-out infinite; }

  /* Bento card */
  .bento-card {
    border-radius: var(--radius-xl, 0.75rem);
    border: 1px solid var(--color-border-subtle);
    background: var(--color-surface);
    transition: transform 0.2s var(--ease-spring), box-shadow 0.2s var(--ease-out), border-color 0.2s var(--ease-out);
  }
  .bento-card:hover { border-color: var(--color-border); box-shadow: var(--shadow-md); }

  /* Glass panel */
  .app-panel-glass {
    border-radius: 0.75rem;
    border: 1px solid var(--color-border-subtle);
    background: var(--glass-bg);
    backdrop-filter: blur(var(--glass-blur));
    -webkit-backdrop-filter: blur(var(--glass-blur));
  }

  /* Custom scrollbar */
  .scrollbar-thin::-webkit-scrollbar { width: 6px; }
  .scrollbar-thin::-webkit-scrollbar-track { background: transparent; }
  .scrollbar-thin::-webkit-scrollbar-thumb { background: var(--color-border); border-radius: 3px; }
  .scrollbar-thin::-webkit-scrollbar-thumb:hover { background: var(--color-text-faint); }
}
```

## Keyframes

```css
@keyframes shimmer { 100% { transform: translateX(100%); } }
@keyframes gradient-shift {
  0%, 100% { background-position: 0% 50%; }
  50% { background-position: 100% 50%; }
}
@keyframes pulse-ring {
  0% { transform: scale(0.8); opacity: 0.4; }
  100% { transform: scale(2.2); opacity: 0; }
}
@keyframes fade-in { from { opacity: 0; } to { opacity: 1; } }
@keyframes fade-in-up {
  from { opacity: 0; transform: translateY(8px); }
  to { opacity: 1; transform: translateY(0); }
}
@keyframes scale-in {
  from { opacity: 0; transform: scale(0.96); }
  to { opacity: 1; transform: scale(1); }
}
@keyframes slide-in-right {
  from { opacity: 0; transform: translateX(12px); }
  to { opacity: 1; transform: translateX(0); }
}
@keyframes stagger-fade-in { to { opacity: 1; } }
@keyframes indeterminate {
  0% { transform: translateX(-100%); }
  100% { transform: translateX(250%); }
}
@keyframes spin-slow { to { transform: rotate(360deg); } }
@keyframes bounce-subtle {
  0%, 100% { transform: translateY(0); }
  50% { transform: translateY(-3px); }
}

/* Reduced motion — mandatory */
@media (prefers-reduced-motion: reduce) {
  *, *::before, *::after {
    animation-duration: 0.01ms !important;
    animation-iteration-count: 1 !important;
    transition-duration: 0.01ms !important;
  }
}
```

## Tailwind Config Extension

Add new colors to `tailwind.config.js`:

```js
theme: {
  extend: {
    colors: {
      // ...existing colors...
      sky: {
        DEFAULT: 'var(--color-sky)',
        light: 'var(--color-sky-light)',
        bg: 'var(--color-sky-bg)',
      },
      violet: {
        DEFAULT: 'var(--color-violet)',
        light: 'var(--color-violet-light)',
        bg: 'var(--color-violet-bg)',
      },
      amber: {
        DEFAULT: 'var(--color-amber)',
        light: 'var(--color-amber-light)',
        bg: 'var(--color-amber-bg)',
      },
      rose: {
        DEFAULT: 'var(--color-rose)',
        light: 'var(--color-rose-light)',
        bg: 'var(--color-rose-bg)',
      },
      blue: {
        DEFAULT: 'var(--color-blue)',
        light: 'var(--color-blue-light)',
        bg: 'var(--color-blue-bg)',
      },
    },
    animation: {
      'fade-in': 'fade-in 0.4s var(--ease-out) both',
      'fade-in-up': 'fade-in-up 0.5s var(--ease-out) both',
      'scale-in': 'scale-in 0.35s var(--ease-spring) both',
      'slide-in-right': 'slide-in-right 0.3s var(--ease-out) both',
      'spin-slow': 'spin-slow 2s linear infinite',
      'bounce-subtle': 'bounce-subtle 2s ease-in-out infinite',
    },
  },
},
```

## Utility Classes (for colors that Tailwind doesn't auto-generate)

```css
@layer utilities {
  .text-sky { color: var(--color-sky); }
  .text-sky-light { color: var(--color-sky-light); }
  .text-violet { color: var(--color-violet); }
  .text-violet-light { color: var(--color-violet-light); }
  .text-amber-light { color: var(--color-amber-light); }
  .text-rose-light { color: var(--color-rose-light); }
  .text-blue-light { color: var(--color-blue-light); }
  .bg-sky-bg { background: var(--color-sky-bg); }
  .bg-violet-bg { background: var(--color-violet-bg); }
  .bg-amber-bg { background: var(--color-amber-bg); }
  .bg-rose-bg { background: var(--color-rose-bg); }
  .bg-blue-bg { background: var(--color-blue-bg); }
  .shadow-sm-custom { box-shadow: var(--shadow-sm); }
  .shadow-md-custom { box-shadow: var(--shadow-md); }
  .shadow-lg-custom { box-shadow: var(--shadow-lg); }
  .shadow-glow { box-shadow: var(--shadow-glow); }
}
```