import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useCallback, useEffect, useRef, useState } from "react";

import { makeDemoData } from "../lib/demo";
import { UsageFeed, type FeedState } from "../lib/usageFeed";
import type { UsageData } from "../types";

export type { FeedStatus } from "../lib/usageFeed";

const USAGE_EVENT = "token-updated";
const DEMO_TICK_MS = 5_000;
const MIN_SPIN_MS = 450;
const sleep = (ms: number) => new Promise((resolve) => setTimeout(resolve, ms));

export function useUsageData() {
  const inTauri = isTauri();
  const [state, setState] = useState<FeedState>({
    data: null, status: inTauri ? "connecting" : "demo", error: null, revision: 0,
  });
  const [refreshing, setRefreshing] = useState(false);
  const feed = useRef<UsageFeed | null>(null);
  const lifetime = useRef<object | null>(null);
  const refreshingFor = useRef<object | null>(null);

  const demo = useCallback(() => {
    setState((old) => ({ ...old, data: makeDemoData(new Date()), revision: old.revision + 1 }));
  }, []);

  useEffect(() => {
    lifetime.current = {};
    if (!inTauri) {
      demo();
      const timer = window.setInterval(demo, DEMO_TICK_MS);
      return () => { lifetime.current = null; window.clearInterval(timer); };
    }
    const current = new UsageFeed({
      subscribe: (receive) => listen<UsageData>(USAGE_EVENT, (event) => receive(event.payload)),
      load: (forceRescan) => invoke<UsageData>("get_usage_data", { forceRescan }),
    }, setState);
    feed.current = current;
    void current.start();
    return () => { lifetime.current = null; feed.current = null; current.dispose(); };
  }, [inTauri, demo]);

  const refresh = useCallback(async () => {
    const active = lifetime.current;
    if (!active || refreshingFor.current === active) return;
    refreshingFor.current = active;
    setRefreshing(true);
    try {
      await Promise.all([inTauri ? feed.current?.refresh() : demo(), sleep(MIN_SPIN_MS)]);
    } finally {
      if (lifetime.current === active) {
        refreshingFor.current = null;
        setRefreshing(false);
      }
    }
  }, [inTauri, demo]);

  return { ...state, refreshing, refresh };
}
