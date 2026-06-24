# Deterministic Time Reference Pattern

**Problem:** ESLint's `react-hooks/purity` rule (and React's rules of hooks) forbids calling impure functions like `Date.now()` inside `useMemo`. In a dashboard activity chart, you want to bucket records by "days ago" relative to now.

**Bad — triggers lint error:**
```tsx
const activityData = useMemo(() => {
  const now = Date.now(); // ❌ Error: Cannot call impure function during render
  const buckets = Array.from({ length: 7 }, (_, i) => ({
    label: `${i - 6 >= 0 ? "+" : ""}${i - 6}d`,
    value: 0,
  }));
  for (const s of managedSkills) {
    const ageMs = now - s.updated_at;
    const dayIdx = 6 - Math.floor(ageMs / (24 * 60 * 60 * 1000));
    if (dayIdx >= 0 && dayIdx < 7) buckets[dayIdx].value++;
  }
  return buckets;
}, [managedSkills]);
```

**Good — deterministic reference from existing data:**
```tsx
const activityData = useMemo(() => {
  const now = managedSkills.reduce((max, s) => Math.max(max, s.updated_at), 0);
  const buckets = Array.from({ length: 7 }, (_, i) => ({
    label: `${i - 6 >= 0 ? "+" : ""}${i - 6}d`,
    value: 0,
  }));
  for (const s of managedSkills) {
    const ageMs = now - s.updated_at;
    const dayIdx = 6 - Math.floor(ageMs / (24 * 60 * 60 * 1000));
    if (dayIdx >= 0 && dayIdx < 7) buckets[dayIdx].value++;
  }
  return buckets;
}, [managedSkills]);
```

**When to use this pattern:**
- Bucketing records relative to "now" for charts, timelines, or recency displays
- Any `useMemo` computation that needs a current timestamp but should remain deterministic for a given render cycle
- The dataset already carries its own `updated_at`, `created_at`, or similar timestamps

**When this pattern doesn't apply:**
- Live clocks or real-time durations that must update continuously → use `useEffect` + `setInterval` instead
- One-shot timestamps for logs/events → compute in an event handler, not in render

**Fallback if no data timestamp exists:**
Move the computation into a `useEffect` that runs in response to an external event or interval, then store the result with `setState` from within the effect body. This keeps render pure while still allowing wall-clock time to drive the UI.
