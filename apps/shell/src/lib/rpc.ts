import { invoke } from "@tauri-apps/api/core";

/** Forward an NDJSON-RPC call to ratd via the src-tauri proxy. */
export async function rpc<T>(method: string, params: unknown = null): Promise<T> {
  return await invoke<T>("rpc_call", { method, params });
}

/** True when the daemon answered the last health probe. */
export async function daemonOk(): Promise<boolean> {
  try {
    await rpc<unknown>("status");
    return true;
  } catch {
    return false;
  }
}

/**
 * Poll `fn` every `ms`, pausing while the document is hidden.
 * Returns a stop function.
 */
export function poll(fn: () => void | Promise<void>, ms: number): () => void {
  let timer: ReturnType<typeof setInterval> | null = null;
  const tick = () => void Promise.resolve(fn()).catch(() => {});
  const start = () => {
    if (timer === null) {
      tick();
      timer = setInterval(tick, ms);
    }
  };
  const stop = () => {
    if (timer !== null) {
      clearInterval(timer);
      timer = null;
    }
  };
  const onVis = () => (document.hidden ? stop() : start());
  document.addEventListener("visibilitychange", onVis);
  start();
  return () => {
    stop();
    document.removeEventListener("visibilitychange", onVis);
  };
}

export function fmtAgo(tsMs: number, now = Date.now()): string {
  const s = Math.max(0, Math.floor((now - tsMs) / 1000));
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86400) return `${Math.floor(s / 3600)}h`;
  return `${Math.floor(s / 86400)}d`;
}

export function fmtDuration(ms: number): string {
  const m = Math.floor(ms / 60000);
  if (m < 60) return `${m} min`;
  return `${Math.floor(m / 60)}h${String(m % 60).padStart(2, "0")}`;
}
