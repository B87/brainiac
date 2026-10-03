/**
 * Settings → Accounts (SPEC.md, Accounts): what each provider asks for and
 * how an account is described.
 */
import type { ForgeAccount, ForgeKind, ForgeTokenKind } from "./ipc";

export type Provider = {
  kind: ForgeKind;
  name: string;
  /** The Keychain item a token can also be added to from Terminal. */
  keychainItem: string;
  /** Where a token is created. */
  tokenUrl: string;
  tokenLabel: string;
  /** What the token needs, in the provider's own words. */
  needs: string;
  needsEmail: boolean;
};

export const PROVIDERS: Provider[] = [
  {
    kind: "github",
    name: "GitHub",
    keychainItem: "brainiac/github",
    tokenUrl: "https://github.com/settings/personal-access-tokens/new",
    tokenLabel: "Fine-grained personal access token",
    needs:
      "Repository permissions: Pull requests read and write, Commit statuses read, Checks read, and Contents read. Merging and resolving threads also need Contents read and write, which also lets the token push code.",
    needsEmail: false,
  },
  {
    kind: "bitbucket_cloud",
    name: "Bitbucket Cloud",
    keychainItem: "brainiac/bitbucket",
    tokenUrl: "https://id.atlassian.com/manage-profile/security/api-tokens",
    tokenLabel: "API token",
    needs:
      "An API token with scopes, for Bitbucket: read:user:bitbucket, read:repository:bitbucket, read:pullrequest:bitbucket, and write:pullrequest:bitbucket. App passwords no longer work.",
    needsEmail: true,
  },
];

export function provider(kind: ForgeKind): Provider {
  return PROVIDERS.find((p) => p.kind === kind) ?? PROVIDERS[0];
}

const TOKEN_KINDS: Record<ForgeTokenKind, string> = {
  fine_grained: "Fine-grained token",
  classic: "Classic token",
  api_token: "API token",
  other: "Token",
};

/** "Fine-grained token · expires 2 Oct 2027", from what the last check found. */
export function tokenSummary(account: ForgeAccount, now = Date.now()): string {
  const parts = [TOKEN_KINDS[account.token_kind]];
  if (account.expires_at) {
    const t = Date.parse(account.expires_at);
    if (!Number.isNaN(t)) {
      const date = new Date(t).toLocaleDateString(undefined, {
        day: "numeric",
        month: "short",
        year: "numeric",
      });
      parts.push(t < now ? `expired ${date}` : `expires ${date}`);
    }
  }
  return parts.join(" · ");
}

/** What the account allows, and what to add to the token for the rest. */
export function accessSummary(account: ForgeAccount): string {
  if (!account.read_only) return "Reads, reviews, and merges pull requests.";
  const missing = account.missing.length
    ? ` To review and merge, add ${account.missing.join(", ")} to the token and replace it.`
    : "";
  return `Read-only: shows pull requests without review and merge actions.${missing}`;
}
