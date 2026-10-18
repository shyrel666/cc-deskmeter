import type { UsageData } from "../types";

export type FeedStatus = "connecting" | "live" | "partial" | "demo" | "error";

export interface FeedState {
  data: UsageData | null;
  status: FeedStatus;
  error: string | null;
  revision: number;
}

export interface UsageTransport {
  subscribe: (receive: (data: UsageData) => void) => Promise<() => void>;
  load: (forceRescan: boolean) => Promise<UsageData>;
}

interface RetryScheduler {
  schedule: (callback: () => void, delayMs: number) => () => void;
}

const timers: RetryScheduler = {
  schedule(callback, delayMs) {
    const id = setTimeout(callback, delayMs);
    return () => clearTimeout(id);
  },
};

/** Snapshots are ordered by the store, independently of command/event delivery
 * order and clock changes. Each mounted consumer owns one feed lifecycle. */
export class UsageFeed {
  private data: UsageData | null = null;
  private revision = 0;
  private disposed = false;
  private unlisten?: () => void;
  private connecting?: Promise<void>;
  private subscriptionError: string | null = null;
  private loadError: string | null = null;
  private loadSerial = 0;
  private cancelRetry?: () => void;
  private retryDelay = 1_000;

  constructor(
    private transport: UsageTransport,
    private changed: (state: FeedState) => void,
    private scheduler: RetryScheduler = timers,
  ) {}

  async start() {
    // Subscribe first so no update can fall between initial load and listen.
    await this.connect();
    await this.load(false);
  }

  async refresh() {
    await this.connect();
    await this.load(true);
  }

  dispose() {
    this.disposed = true;
    this.cancelRetry?.();
    this.unlisten?.();
    this.unlisten = undefined;
  }

  private connect(): Promise<void> {
    if (this.disposed || this.unlisten) return Promise.resolve();
    if (this.connecting) return this.connecting;
    this.cancelRetry?.();
    this.cancelRetry = undefined;
    this.connecting = this.subscribe().finally(() => { this.connecting = undefined; });
    return this.connecting;
  }

  private async subscribe() {
    try {
      const unlisten = await this.transport.subscribe((next) => {
        if (this.accept(next)) {
          this.loadError = null;
          this.publish();
        }
      });
      if (this.disposed) { unlisten(); return; }
      this.unlisten = unlisten;
      this.subscriptionError = null;
      this.retryDelay = 1_000;
    } catch (error) {
      if (this.disposed) return;
      this.subscriptionError = `Live updates unavailable: ${String(error)}`;
      const delay = this.retryDelay;
      this.retryDelay = Math.min(delay * 2, 30_000);
      this.cancelRetry = this.scheduler.schedule(() => {
        this.cancelRetry = undefined;
        // Reload events missed while disconnected, even if no new event lands.
        void this.connect().then(() => this.load(false));
      }, delay);
    }
    this.publish();
  }

  private async load(forceRescan: boolean) {
    if (this.disposed) return;
    const request = ++this.loadSerial;
    const before = this.data?.snapshotId;
    try {
      const next = await this.transport.load(forceRescan);
      if (this.accept(next)) {
        this.loadError = null;
        this.publish();
      }
    } catch (error) {
      // A late rejection cannot undo a newer successful command or event.
      if (this.disposed || request !== this.loadSerial || before !== this.data?.snapshotId) return;
      this.loadError = error instanceof Error ? error.message : String(error);
      this.publish();
    }
  }

  private accept(next: UsageData): boolean {
    if (this.disposed || (this.data && next.snapshotId <= this.data.snapshotId)) return false;
    this.data = next;
    this.revision += 1;
    return true;
  }

  private publish() {
    if (this.disposed) return;
    const error = [this.subscriptionError, this.loadError].filter(Boolean).join("\n") || null;
    const status: FeedStatus = error ? "error"
      : !this.unlisten || !this.data ? "connecting"
      : this.data.readErrors.length > 0 ? "partial" : "live";
    this.changed({ data: this.data, revision: this.revision, status, error });
  }
}
