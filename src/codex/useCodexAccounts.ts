import { useCallback, useEffect, useMemo, useSyncExternalStore } from "react";

import { useI18n } from "../i18n/I18nProvider";
import { assignSlots, emailNames, parseStoredAccounts, renameAccount, type StoredAccounts } from "./accounts";
import { useCodexPalette } from "./palette";
import type { CodexAccount } from "./types";

const STORAGE_KEY = "claude-pulse.codexAccounts";

// One record for every hook in this window; the app's other windows (the
// tray panel) follow through `storage` events, as for the theme and language.
const listeners = new Set<() => void>();
let snapshot: { raw: string | null; accounts: StoredAccounts } | null = null;

function storedRaw(): string | null {
  try {
    return localStorage.getItem(STORAGE_KEY);
  } catch {
    // Storage can be unavailable (private mode, locked down webview): keep what this window set.
    return snapshot?.raw ?? null;
  }
}

function getSnapshot(): StoredAccounts {
  const raw = storedRaw();
  if (!snapshot || snapshot.raw !== raw) snapshot = { raw, accounts: parseStoredAccounts(raw) };
  return snapshot.accounts;
}

function save(accounts: StoredAccounts) {
  const raw = JSON.stringify(accounts);
  snapshot = { raw, accounts };
  try {
    localStorage.setItem(STORAGE_KEY, raw);
  } catch {
    // Not fatal: names and colours just won't persist.
  }
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  const onStorage = (event: StorageEvent) => {
    if (event.key === STORAGE_KEY || event.key === null) listener();
  };
  window.addEventListener("storage", onStorage);
  return () => {
    listeners.delete(listener);
    window.removeEventListener("storage", onStorage);
  };
}

/** How an account is shown, wherever it is. */
export interface AccountLook {
  /** The name the user gave it, else one from its email, else `Account N` by its place among `accounts`. */
  name: string;
  /** Whether `name` is one the user gave. */
  named: boolean;
  color: string;
}

/**
 * The accounts' names and colours, the same in every view and window: each
 * keeps the colour it was first given, stored with its name by short id.
 */
export function useCodexAccounts(accounts: CodexAccount[]) {
  const { t } = useI18n();
  const palette = useCodexPalette();
  const stored = useSyncExternalStore(subscribe, getSnapshot);
  const ids = accounts.map((account) => account.shortId).join("\n");
  const { slots, next } = useMemo(() => assignSlots(stored, ids ? ids.split("\n") : []), [stored, ids]);
  // A new account's slot is stored once it is drawn, so the colour stays its own.
  useEffect(() => {
    if (next) save(next);
  }, [next]);

  const looks = useMemo<AccountLook[]>(() => {
    const defaults = emailNames(accounts.map((account) => account.email));
    return accounts.map((account, index) => {
      const alias = stored[account.shortId]?.alias;
      const name = alias ?? defaults[index] ?? t.codex.account(index + 1);
      return { name, named: alias !== undefined, color: palette.accounts[slots[index]] };
    });
  }, [accounts, stored, slots, palette, t]);
  /** A blank name restores the default. */
  const rename = useCallback((shortId: string, alias: string) => save(renameAccount(getSnapshot(), shortId, alias)), []);
  return { looks, rename };
}
