export function formatDuration(ms: number | null | undefined): string {
  if (ms === null || ms === undefined || ms < 0) return "—";
  if (ms < 1000) return `${ms} мс`;
  const sec = Math.round(ms / 1000);
  if (sec < 60) return `${sec} сек`;
  const min = Math.floor(sec / 60);
  const remSec = sec - min * 60;
  if (min < 60) {
    if (min < 10 && remSec > 0) return `${min} мин ${remSec} сек`;
    return `${min} мин`;
  }
  const hr = Math.floor(min / 60);
  const remMin = min - hr * 60;
  if (hr < 24) return remMin > 0 ? `${hr} ч ${remMin} мин` : `${hr} ч`;
  const days = Math.floor(hr / 24);
  const remHr = hr - days * 24;
  return remHr > 0 ? `${days} д ${remHr} ч` : `${days} д`;
}

export function formatAgo(ts: number | null | undefined): string {
  if (!ts) return "—";
  const delta = Date.now() - ts;
  if (delta < 30_000) return "только что";
  return formatDuration(delta) + " назад";
}

export function formatDateTime(ts: number | null | undefined): string {
  if (!ts) return "—";
  const d = new Date(ts);
  return d.toLocaleString("ru-RU", {
    day: "numeric",
    month: "short",
    hour: "2-digit",
    minute: "2-digit",
  });
}
