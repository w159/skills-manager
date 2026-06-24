---
name: app-ui-overhaul
description: "Systematically enhance an existing production app's UI/UX: extend the CSS design system, build a modular component library, add loading states, and redesign key views — without breaking existing functionality. Covers Tauri/Electron desktop apps and web apps with React/Vue/Svelte + Tailwind or CSS variables."
version: 1.0.0
author: Hermes Agent
license: MIT
platforms: [linux, macos, windows]
metadata:
  hermes:
    tags: [ui, ux, design-system, components, react, tailwind, tauri, desktop, loading-states, animation, overhaul]
    related_skills: [claude-design, sketch, popular-web-designs, design-md]
---

# App UI Overhaul

Use this skill when the user asks to dramatically improve the UI/UX of an **existing production app** — not a one-off HTML artifact (`claude-design`), not a throwaway mockup (`sketch`), and not matching a known brand (`popular-web-designs`). This is the skill for taking a functional-but-plain app and making it look incredible while preserving all existing behavior.

## When To Use

- "Make the UI/UX incredible for this app"
- "Redesign the dashboard with better visuals"
- "Add loading screens, animations, and modern UI components"
- "Overhaul the design system for this desktop app"
- "Create modular UI components for this project"

## When NOT To Use

- One-off HTML artifacts (landing pages, decks, prototypes) → `claude-design`
- Throwaway mockups to compare design directions → `sketch`
- Matching a known brand's visual style → `popular-web-designs`
- Authoring a DESIGN.md token spec file → `design-md`
- Building from scratch with no existing codebase → just build it well

## How It Differs From Sibling Skills

| Skill | Starting point | Deliverable |
|-------|---------------|-------------|
| `claude-design` | Blank canvas or brief | Self-contained HTML artifact |
| `sketch` | Design question | 2-3 disposable HTML variants for comparison |
| `popular-web-designs` | "Make it look like Stripe" | Pasted design system values |
| **app-ui-overhaul** (this) | Existing production codebase | Enhanced codebase, same functionality, dramatically better UI |

The key constraint: **the app must still build and work after your changes.** This is production code, not a prototype.

## Workflow

### 1. Analyze the Existing Codebase (Required — Never Skip)

Before writing any CSS or components, read the actual source:

- `tailwind.config.js` / `tailwind.config.ts` — existing color tokens, fonts, plugins
- `src/index.css` or global stylesheet — CSS variables, theme tokens, component classes
- `src/App.tsx` or root component — routing, layout structure
- `src/components/Layout.tsx` — shell, sidebar, content area structure
- `src/context/` — app state shape (what data is available)
- `src/lib/api.ts` or equivalent — backend API surface (Tauri invoke, fetch, etc.)
- Key views: Dashboard, Settings, Library/List views
- `package.json` — dependencies (animation libs? icon sets? DnD?)

**Do not design from memory when source files are available.** Read the theme, read the tokens, read the components. The file tree is the menu — read the files that define the visual vocabulary.

### 2. Extend the CSS Design System

Add to the existing design system, don't replace it:

**Color palette extension:**
```css
/* Add semantic colors alongside existing accent/danger */
--color-sky: #0EA5E9;
--color-sky-light: #38BDF8;
--color-sky-bg: rgba(14,165,233,0.08);
--color-violet: #8B5CF6;
--color-violet-light: #A78BFA;
--color-violet-bg: rgba(139,92,246,0.08);
/* ... amber, rose, blue ... */
```

**Dark mode parity:** Every new color token must have a `.dark` override with desaturated/lighter variants.

**Shadow tokens:**
```css
--shadow-sm: 0 1px 2px rgba(0,0,0,0.04);
--shadow-md: 0 2px 8px rgba(0,0,0,0.06);
--shadow-lg: 0 8px 24px rgba(0,0,0,0.08);
--shadow-glow: 0 0 20px rgba(16,185,129,0.12);
```

**Animation tokens:**
```css
--ease-spring: cubic-bezier(0.34, 1.56, 0.64, 1);
--ease-out: cubic-bezier(0.16, 1, 0.3, 1);
--ease-in-out: cubic-bezier(0.65, 0, 0.35, 1);
```

**Glass morphism:**
```css
--glass-blur: 12px;
--glass-bg: rgba(255,255,255,0.72);
--glass-border: rgba(255,255,255,0.18);
```

### 3. Add CSS Utility Classes and Keyframes

Add these patterns to `@layer components` and raw CSS:

| Class | Purpose |
|-------|---------|
| `.skeleton` | Shimmer placeholder with animated gradient sweep |
| `.gradient-text` | Animated 3-color gradient text clip |
| `.pulse-dot` | Expanding ring pulse for live indicators |
| `.animate-fade-in` | 0.4s fade-in entrance |
| `.animate-fade-in-up` | 0.5s fade + translateY entrance |
| `.animate-scale-in` | 0.35s spring scale entrance |
| `.stagger-children > *` | 40ms incremental animation delays (up to 10 items) |
| `.hover-lift` | translateY(-2px) + shadow on hover |
| `.progress-bar` / `.progress-bar-fill` | Animated progress bar with indeterminate mode |
| `.bento-card` | Rounded border card with hover elevation |
| `.scrollbar-thin` | Custom themed scrollbar |

**Keyframes needed:** `shimmer`, `gradient-shift`, `pulse-ring`, `fade-in`, `fade-in-up`, `scale-in`, `slide-in-right`, `stagger-fade-in`, `indeterminate`, `spin-slow`, `bounce-subtle`

**Reduced motion is mandatory:**
```css
@media (prefers-reduced-motion: reduce) {
  *, *::before, *::after {
    animation-duration: 0.01ms !important;
    animation-iteration-count: 1 !important;
    transition-duration: 0.01ms !important;
  }
}
```

### 4. Update Tailwind Config

Add new colors to `tailwind.config.js` theme.extend.colors so Tailwind utilities pick them up:

```js
sky: {
  DEFAULT: 'var(--color-sky)',
  light: 'var(--color-sky-light)',
  bg: 'var(--color-sky-bg)',
},
// ... violet, amber, rose, blue
```

### 5. Build a Modular UI Component Library

Create `src/components/ui.tsx` (or equivalent) with reusable components. See `references/component-patterns.md` for full code.

Core components:

| Component | What it does |
|-----------|-------------|
| `SkeletonText` | Multi-line shimmer placeholder |
| `SkeletonCard` | Card-shaped skeleton for grid loading |
| `SkeletonRow` | List-row skeleton for list loading |
| `SkeletonGrid` / `SkeletonList` | Pre-built skeleton layouts |
| `StatRing` | Circular SVG progress ring with animated stroke-dashoffset |
| `StatCard` | Bento-grid stat card with icon or ring, animated entrance, hover lift |
| `StatusBadge` | Pill badge with 7 tones (default, success, warning, danger, info, violet, neutral) + optional pulsing dot |
| `EmptyState` | Icon in rounded container + title + description + action button |
| `ProgressBar` | Determinate or indeterminate animated bar |
| `SectionHeader` | Title + subtitle + action slot |
| `IconBadge` | Colored icon container with semantic coloring |
| `MiniBarChart` | CSS-only bar chart with staggered entrance |
| `SyncStatusIndicator` | 5-state status with colored pulsing dot |
| `AnimatedCounter` | requestAnimationFrame-based count-up |

### 6. Build a Splash/Loading Screen

Create a phased loading screen that honestly reports what the backend is doing:

```tsx
const PHASES = [
  { label: "Initializing", detail: "Starting up the engine…", minMs: 200 },
  { label: "Loading presets", detail: "Fetching saved configurations…", minMs: 150 },
  { label: "Detecting agents", detail: "Scanning for installed tools…", minMs: 150 },
  { label: "Reading library", detail: "Loading central repository…", minMs: 200 },
  { label: "Ready", detail: "Everything loaded.", minMs: 100 },
];
```

Each phase label must correspond to an actual backend operation. Do not fake phases — if the app loads data in parallel, say so. The splash screen fades out when data is ready.

Integrate into the layout:
```tsx
{(!splashDone || loading) && <SplashScreen onComplete={() => setSplashDone(true)} />}
```

### 7. Redesign Key Views

**Dashboard:** Bento-grid stat cards with animated counters, circular SVG progress ring for sync/health, mini bar chart for activity, source-type breakdown bars, connected-agents grid, recent-items list with per-type colored icons, empty state.

**Sidebar:** Logo with gradient glow blur, per-asset-type color coding in bordered icon boxes, smooth fade-in animation on items, settings link with matching icon container treatment.

**Asset Library / List Views:** Colored tab bar with per-type semantic colors, skeleton loading states (not spinners), error states with icon badge, enhanced empty states with action buttons, staggered list entrance, hover elevation on rows.

**Sync/Status Indicators:** Hover scale animation on interactive dots, pulsing dots for active states, colored status badges.

### 8. Per-Type Color Coding Pattern

When an app has multiple asset/category types, assign each a semantic color:

```ts
const TYPE_COLORS = {
  skill:     { color: "var(--color-accent-light)", bg: "var(--color-accent-bg)" },
  agent:     { color: "var(--color-sky-light)",    bg: "var(--color-sky-bg)" },
  command:   { color: "var(--color-violet-light)",  bg: "var(--color-violet-bg)" },
  hook:      { color: "var(--color-amber-light)",   bg: "var(--color-amber-bg)" },
  script:    { color: "var(--color-blue-light)",   bg: "var(--color-blue-bg)" },
  rule:      { color: "var(--color-rose-light)",   bg: "var(--color-rose-bg)" },
  // ...
};
```

Use these colors consistently in sidebar icons, tab bars, dashboard cards, and list rows. The color assignment should be stable across the app — the same type always gets the same color.

### 9. Verify the Build

After all changes:

1. **TypeScript:** `npx tsc -b` — must pass with zero errors
2. **Vite/Webpack build:** `npx vite build` or equivalent — must succeed
3. **ESLint:** `npx eslint src/` — fix any new issues introduced
4. **Manual check:** If possible, run the dev server and visually verify

Common issues to fix:
- Unused imports after refactoring (TS6133)
- `React.ReactNode` vs `string | number` type mismatches when using JSX as prop values
- ESLint `prefer-const` on arrays that are only pushed to
- ESLint `react-hooks/purity` rules flagging `Date.now()` inside `useMemo`. **Avoid suppressing it.** Instead derive the reference time deterministically from existing data (e.g. `managedSkills.reduce((max, s) => Math.max(max, s.updated_at), 0)`). If there is truly no state to anchor to, move the timestamp logic into a `useEffect` that calls `setState` from an event callback rather than `useMemo`.
- ESLint `@typescript-eslint/no-explicit-any` — use proper typed Record instead of `as any`

## Pitfalls

- **Do not break existing functionality.** Every change must preserve the app's current behavior. The build must pass.
- **Do not replace the design system — extend it.** Add new tokens alongside existing ones. Existing component classes (`.app-panel`, `.app-input`, etc.) should keep working.
- **Do not force a standalone HTML artifact.** When the user has a real repo with a real stack, work in that stack. Do not create a separate HTML file.
- **Do not use spinners for loading states > 1s.** Use skeleton screens instead — they reduce perceived wait time and show the content shape.
- **Do not animate width/height/top/left.** Use `transform` and `opacity` only for performant animations.
- **Do not forget `prefers-reduced-motion`.** Every animation must degrade gracefully.
- **Do not use emoji as icons.** Use the project's existing icon library (lucide-react, heroicons, etc.).
- **Do not invent fake data for dashboards.** Use real data from the app's state/context. If no data exists, show an empty state.
- **Do not skip the dark mode.** Every new color token needs a `.dark` override. Test both themes.
- **Do not forget to import React** when using hooks like `useState` in component library files that also export non-hook components.
- **StatCard `value` prop should accept `React.ReactNode`**, not just `string | number`, so you can pass `<AnimatedCounter>` JSX elements.
- **Tauri apps on macOS:** The top 28px is a drag region for window controls. Do not place content above `pt-[28px]` or it will be under the traffic lights.

## Verification Checklist

The user directive "finish what you were working on" applies here: do not declare the job done until the artifact is verified. After all changes:

1. **TypeScript:** `npx tsc -b` (or `npx tsc --noEmit`) — must pass with zero errors
2. **Vite/Webpack build:** `npx vite build` — must succeed
3. **ESLint:** `npx eslint src/` — fix any new issues introduced
4. **Run the dev server** if a server is part of the app (`npm run dev`, `npm run tauri dev`, etc.) and visually verify the splash screen and dashboard

If the user explicitly says "finish what you were working on" or similar, treat it as a workflow correction: immediately return to verification and complete it before summarizing.

## Reference Files

- `references/component-patterns.md` — Full code for all modular UI components
- `references/css-patterns.md` — Full CSS for all keyframes, utility classes, and design tokens
- `references/deterministic-time-pattern.md` — How to avoid `Date.now()` inside `useMemo` while still building time-bucketed charts