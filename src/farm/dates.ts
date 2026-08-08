// Parse a local YYYY-MM-DD date. Do not use `new Date("YYYY-MM-DD")` —
// that parses as UTC midnight and can render the previous day west of Greenwich.
export function parseLocalDate(iso: string): Date {
  const [y, m, d] = iso.split("-").map(Number);
  return new Date(y, m - 1, d);
}

export function addDays(d: Date, days: number): Date {
  return new Date(d.getFullYear(), d.getMonth(), d.getDate() + days);
}

export function localToday(): Date {
  const n = new Date();
  return new Date(n.getFullYear(), n.getMonth(), n.getDate());
}

// -> "ready Fri Aug 14"   (no comma)
export function readyLabel(d: Date): string {
  const parts = new Intl.DateTimeFormat("en-US", {
    weekday: "short",
    month: "short",
    day: "numeric",
  }).formatToParts(d);

  const weekday = parts.find((p) => p.type === "weekday")?.value ?? "";
  const month = parts.find((p) => p.type === "month")?.value ?? "";
  const day = parts.find((p) => p.type === "day")?.value ?? "";

  return `ready ${weekday} ${month} ${day}`;
}

// -> "Saturday"
export function weekdayName(d: Date): string {
  return new Intl.DateTimeFormat("en-US", { weekday: "long" }).format(d);
}

/** "6:14 pm" — lowercase am/pm, no leading zero on the hour. */
export function formatClock(d: Date): string {
  const parts = new Intl.DateTimeFormat("en-US", {
    hour: "numeric",
    minute: "2-digit",
    hour12: true,
  }).formatToParts(d);
  const hour = parts.find((p) => p.type === "hour")?.value ?? "";
  const minute = parts.find((p) => p.type === "minute")?.value ?? "";
  const dayPeriod = (
    parts.find((p) => p.type === "dayPeriod")?.value ?? ""
  ).toLowerCase();
  return `${hour}:${minute} ${dayPeriod}`;
}

/**
 * Snapshot list labels: "Today at 6:14 pm", "Yesterday at 7:02 pm",
 * "Monday at 6:30 pm".
 */
export function snapshotLabel(d: Date, now: Date = new Date()): string {
  const time = formatClock(d);
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const day = new Date(d.getFullYear(), d.getMonth(), d.getDate());
  const diffDays = Math.round(
    (today.getTime() - day.getTime()) / (24 * 60 * 60 * 1000),
  );
  if (diffDays === 0) return `Today at ${time}`;
  if (diffDays === 1) return `Yesterday at ${time}`;
  return `${weekdayName(d)} at ${time}`;
}
