import { memo } from "react";

import { AccountCards } from "./AccountCards";
import { BreakdownCards } from "./BreakdownCards";
import { OverviewCard } from "./OverviewCard";
import { ThreadBoard } from "./ThreadBoard";
import { TrendChart } from "./TrendChart";
import type { CodexData } from "./types";
import { useCodexAccounts } from "./useCodexAccounts";

/**
 * The Codex dashboard, tokens first everywhere: what was used, each
 * account's official limits, the trend, where the week's tokens went and the
 * thread board. Every card fills its row or matches its neighbours' height.
 */
export const CodexView = memo(function CodexView({ data }: { data: CodexData }) {
  const { looks, rename } = useCodexAccounts(data.accounts);
  // Entrance slots: the overview, up to three account cards, then the rest in order.
  const after = 2 + Math.min(Math.max(data.accounts.length, 1), 3);
  return (
    <div className="space-y-4">
      <OverviewCard data={data} />
      <AccountCards data={data} looks={looks} onRename={rename} slot={2} />
      <TrendChart data={data} looks={looks} slot={after} />
      <BreakdownCards data={data} slot={after + 1} />
      <ThreadBoard data={data} looks={looks} slot={after + 4} />
    </div>
  );
});
