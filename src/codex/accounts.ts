// What the app remembers of each Codex account: the name the user gave it
// (else one from its email) and its colour, kept by shortened id; neither
// ever leaves this machine's storage.

/** Colours in each theme's account palette (`CodexPalette.accounts`). */
export const ACCOUNT_SLOTS = 5;
/** Longest name kept. */
export const ALIAS_MAX = 24;

export interface StoredAccount {
  /** The name the user gave; absent while the account has its default. */
  alias?: string;
  /** Its colour: an index into the palette's accounts, kept once given. */
  slot?: number;
}

/** By `CodexAccount.shortId`. */
export type StoredAccounts = Record<string, StoredAccount>;

const isSlot = (value: unknown): value is number =>
  typeof value === "number" && Number.isInteger(value) && value >= 0 && value < ACCOUNT_SLOTS;

/** The stored record, leaving out whatever is malformed. */
export function parseStoredAccounts(raw: string | null): StoredAccounts {
  let parsed: unknown;
  try {
    parsed = JSON.parse(raw ?? "{}");
  } catch {
    return {};
  }
  if (typeof parsed !== "object" || parsed === null || Array.isArray(parsed)) return {};
  const accounts: StoredAccounts = {};
  for (const [id, entry] of Object.entries(parsed)) {
    if (typeof entry !== "object" || entry === null) continue;
    const { alias, slot } = entry as Record<string, unknown>;
    const kept: StoredAccount = {};
    if (typeof alias === "string" && alias.trim()) kept.alias = alias.trim().slice(0, ALIAS_MAX);
    if (isSlot(slot)) kept.slot = slot;
    if (kept.alias !== undefined || kept.slot !== undefined) accounts[id] = kept;
  }
  return accounts;
}

/**
 * Each account's colour slot, in `ids` order, and the record to store when
 * that gave any account a new one (else null). An account keeps its slot; a
 * new one takes the lowest slot no account ever took, then the lowest one
 * left by an account no longer read (which gives it up), so colours never
 * follow the order the accounts are read in. Past the palette, accounts
 * share slots in turn, unstored, until one frees up.
 */
export function assignSlots(stored: StoredAccounts, ids: string[]): { slots: number[]; next: StoredAccounts | null } {
  const present = new Set(ids);
  const held = new Set<number>();
  // Two accounts on one slot (two windows assigning at once): the first keeps it.
  const kept = ids.map((id) => {
    const slot = stored[id]?.slot;
    if (!isSlot(slot) || held.has(slot)) return null;
    held.add(slot);
    return slot;
  });
  const everHeld = new Set(Object.values(stored).map((entry) => entry.slot).filter(isSlot));
  const all = [...Array(ACCOUNT_SLOTS).keys()];
  let next: StoredAccounts | null = null;
  const slots: number[] = [];
  for (const [index, id] of ids.entries()) {
    const keptSlot = kept[index];
    if (keptSlot !== null) {
      slots.push(keptSlot);
      continue;
    }
    const slot = all.find((s) => !everHeld.has(s)) ?? all.find((s) => !held.has(s));
    if (slot === undefined) {
      slots.push(index % ACCOUNT_SLOTS);
      continue;
    }
    next ??= { ...stored };
    // An account no longer read gives up the slot it held.
    for (const [other, entry] of Object.entries(next)) {
      if (entry.slot !== slot || present.has(other)) continue;
      if (entry.alias) next[other] = { alias: entry.alias };
      else delete next[other];
    }
    next[id] = { ...next[id], slot };
    held.add(slot);
    everHeld.add(slot);
    slots.push(slot);
  }
  return { slots, next };
}

/**
 * Default names, in `emails` order: the part of each email before the `@`,
 * or the whole email where two parts read alike; null without an email.
 */
export function emailNames(emails: (string | null)[]): (string | null)[] {
  const parts = emails.map((email) => email?.split("@")[0]?.trim().toLowerCase() || null);
  return emails.map((email, index) => {
    const part = parts[index];
    if (!email || !part) return null;
    const clash = parts.some((other, j) => j !== index && other === part);
    return clash ? email : email.split("@")[0].trim();
  });
}

/** `stored` with `id` named `alias`; a blank name restores the default. */
export function renameAccount(stored: StoredAccounts, id: string, alias: string): StoredAccounts {
  const name = alias.trim().slice(0, ALIAS_MAX);
  const entry: StoredAccount = { ...stored[id] };
  if (name) entry.alias = name;
  else delete entry.alias;
  const next = { ...stored };
  if (entry.alias === undefined && entry.slot === undefined) delete next[id];
  else next[id] = entry;
  return next;
}
