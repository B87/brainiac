# Design: database connections and queries

Design notes for a database client inside Brainiac: saved connections to SQLite files and PostgreSQL servers, a query editor with a fast result grid, and saved queries for the ones used every week. The behavior is specified in [`SPEC.md`](../../SPEC.md) section 11 and the design in [`architecture.md`](../architecture.md), Databases — v0.4; this file keeps the background and open questions.

They reuse what earlier releases already settled: all network and file access runs in Rust and the WebView only calls commands (`architecture.md`, Security); secrets live only in the Keychain, behind a token type that cannot reach a log (`architecture.md`, Pull requests — v0.3, Identity); every write goes through a domain service with an expected version; `brainiac.db` holds what cannot be rebuilt and `history.db` holds optional recovery data (`architecture.md`, Storage layout — v0.2).

## Status: in v0.4 (4 Oct 2026)

Built for v0.4 on the `v0.4-databases` branch. What the user sees moved to [`SPEC.md`](../../SPEC.md) section 11 and how it is built to [`architecture.md`](../architecture.md), Databases — v0.4, whose Decisions also hold the spike's results. This file keeps the background: why, the options compared, what was left out, and what is still open. The visual design was drawn on a canvas as three layouts; option B was built.

## Why

A database GUI is open most of the day: checking a row while debugging, answering "how many customers did X", running the same five reporting queries every Monday. The usual tools are heavy (DataGrip), paid (TablePlus), or slow and noisy (pgAdmin). What a programmer wants from one is small:

- Connections that are quick to switch and hard to confuse, especially production.
- An editor with completion for the schema in front of it, where `Cmd+Enter` runs the statement under the cursor.
- A grid that stays fast with thousands of rows and copies cleanly into a note, a ticket, or Slack.
- The queries used every week, saved with a name, one keystroke away.
- No accidental writes to production.

## Design it twice

### Layout

Three layouts were drawn (4 Oct 2026):

| | A. Navigator | B. Editor first (chosen) | C. Console |
| --- | --- | --- | --- |
| Shape | A tree of every connection, its schema, saved queries, and history beside the editor | A Home page for connections and saved queries; tabs; a side panel with the tab's own connection's schema, saved queries, and history | One transcript per connection, input at the bottom; saved queries are `/commands`, schema is `\d` |
| Strength | Everything visible at once | The editor gets the width; the panel shows only what matters to the tab | Fastest from the keyboard; the transcript is the history, and an open transaction is drawn as one rail |
| Cost | A second sidebar beside the app's; most of it is about connections the tab is not using | Comparing two connections' schemas takes two tabs | Long queries are cramped in the input; comparing results means scrolling |

B fits how a database is used day to day: one connection at a time, with saved queries reached from `Cmd+K` or Home more often than browsed. C's `/commands` could still come later as a way to run saved queries from the editor.

### Where saved queries live

| | A. Records in `brainiac.db` (chosen) | B. Notes in the vault | C. `.sql` files in a folder |
| --- | --- | --- | --- |
| Shape | A `saved_queries` table; the query view edits them | A note with `brainiac_query` frontmatter and its first `sql` fence as the query | One file per query in a chosen folder, with a comment header for name and connection |
| Without a vault | Works | Needs a vault, which databases otherwise do not | Works |
| Editing | One editor, with completion and Run | Two editors for the same text, the note editor and the query view, writing the same fence | One editor; outside edits need a watcher |
| Format Brainiac must enforce | None | "First `sql` fence is the query" in a file the user and agents edit freely | A comment header convention |
| Search and links | Added to search like tasks; **Copy as Markdown** for notes | Free: notes are already searched and linked | A new indexer |
| Portability | Export and restore; not readable outside Brainiac | Ordinary Markdown | Ordinary files, usable from `psql -f` |

B fits "notes are ordinary Markdown" best on paper, but it makes the query view a second writer of note files and gives a fence a meaning Brainiac would have to keep when the user or an agent edits around it. Every save would rewrite part of a note, which the vault rules allow only on an explicit action and which the note editor's conflict handling would then have to cover. A keeps one owner and one editor, and covers what B offers that matters day to day: search, and getting a query into a note. C remains a reasonable later addition as an export ("Save to File…").

### PostgreSQL driver

| | `tokio-postgres` (chosen) | `sqlx` | `libpq` through bindings |
| --- | --- | --- | --- |
| Language | Pure Rust | Pure Rust | C library to bundle and sign, with its own TLS |
| Dynamic results | Column type OIDs, and raw values for any type through a custom `FromSql` | Built around compile-time typed queries; dynamic rows are possible but secondary | Text values for everything, the most faithful |
| Row cap without rewriting SQL | Portals with a row limit (`bind` and `query_portal`) | Streams; capping means dropping the stream | Cursors or single-row mode |
| Cancel | `CancelToken` | Through the pool, less direct | `PQcancel` |
| TLS | `tokio-postgres-rustls`, the same `rustls` `reqwest` already uses | rustls or native-tls | OpenSSL inside libpq |

`tokio-postgres` gives exactly the controls a GUI needs (portals, cancel, notices) with the smallest surface and no C library to package. SQLite stays on `rusqlite`, already a dependency, on its own connection per session, never the app's database worker.

## Performance on the connected database

A review on 4 Oct 2026 read the v0.4 code and measured it against a local PostgreSQL 16. The test database had 5,000 tables, about 17,000 indexes, and a table with 1,000 partitions, behind a proxy that added 25 ms to every round trip. This section records what using Brainiac costs a server, what was changed after the review, and what is still open. Hosted providers add their own round-trip time, about 20–50 ms to a Cloud SQL instance from a laptop.

### What a session costs

| Source | Server connections | Lifetime | Queries |
| --- | --- | --- | --- |
| Query tab | One per tab, opened on its first Run (restoring tabs opens none) | Closed when the tab closes or switches connection, or after 10 idle minutes; at most 8 across all connections, but a tab with an open transaction or a running statement is never closed early | Only what the user runs |
| Health | One (`Brainiac health`) while the Health tab is on screen and the window is not minimized or hidden | Dropped within 10 s of the tab being hidden | 5 small queries every 10 s, table sizes every 60 s; two round trips per sample |
| Schema for completion | One per connection, when a tab on screen first uses it | Closed after the catalog read | 6 catalog queries in one read-only transaction; shared by every tab that asks while it runs, and cached per connection version |
| Test, Cancel, End Session | One per click | A moment | — |

- **Round trips per run.** A read-only run takes 4 round trips: `START TRANSACTION READ ONLY`, Parse and Describe, Bind, and Execute. `COMMIT` is sent without waiting for its answer. A result that needs a second batch of 1,000 rows adds one more.
  - Before the change, measured at 25 ms per round trip: about 135 ms of overhead per run, so about 125–300 ms on Cloud SQL. The change saves one round trip, about 20–50 ms on Cloud SQL.
  - A new connection with TLS and SCRAM takes about 7–8 round trips. Its settings and `SHOW server_version` now travel together as one more, where they took three.
- **Nothing stays open in Read only or Auto-commit.** Each run opens and ends its transaction inside the call, so no lock or snapshot outlives it.
- **The portal path stops server work at the cap.** It asks for at most cap + 1 rows, then closes the portal and commits.
  - The exception is a plan that must finish before it returns its first row, such as a sort or a hash aggregate.
- **Things the app avoids:**
  - No polling outside Health.
  - No implicit `SELECT *` previews and no `count(*)`: row estimates come from `reltuples`, and Select rows opens `select … limit 100` without running it.
  - Health reads table sizes from `relpages` rather than `pg_total_relation_size`, so it never waits on locks.
- **Session settings.**
  - Every session:
    - `application_name`;
    - `statement_timeout` (30 s by default, 2 s for Health);
    - `default_transaction_read_only`;
    - server TCP keepalives (60 s idle, 10 s apart, 6 probes), with the client probing the same way;
    - where the server has them (PostgreSQL 14 and later), `idle_session_timeout` (15 minutes) and `client_connection_check_interval` (10 seconds).
  - Writable modes only: `lock_timeout = 5s`, and `idle_in_transaction_session_timeout` of 15 minutes, or 5 on Production.
- **Worst case for one server.** About 8 query sessions, Health, one schema read per connection, and short-lived ones. Small Cloud SQL tiers allow 25–100 connections, so Brainiac alone can take a fifth or more of them.

### Findings

Status as of 4 Oct 2026.

| Impact | What happens on the server | When it matters | Status |
| --- | --- | --- | --- |
| High | **Manual mode holds locks.** After a Manual `SELECT` the backend is `idle in transaction` with `AccessShareLock` on the table. A migration's `ALTER TABLE` queues behind it, and every later query on the table queues behind the migration. After a write, the transaction also holds row locks and holds back VACUUM until Commit, Roll Back, or the server's idle limit. | Production with migrations or busy writers | **Reduced.** On Production the server now ends an idle transaction after 5 minutes rather than 15, and the bar turns amber after 2. The locks themselves are what Manual is for. |
| High in Manual | **A `SELECT` in Manual mode streams the whole result.** Statements in an open transaction use the simple path, which reads and drops every row past the cap until the statement ends or reaches `statement_timeout`. On a hosted server that is a full scan plus all its network egress. Auto-commit's writable retry uses the same path. | Large tables in Manual | **Open.** Read row-returning statements in Manual through a cursor (`DECLARE … NO SCROLL CURSOR`, `FETCH cap+1`, `CLOSE`). |
| Medium | **Restored tabs loaded the schema at once.** Every tab, hidden ones included, asked for its connection's schema, and nothing deduplicated loads already in flight. N tabs on one connection opened N connections that each read the whole catalog: about 450 ms of server time and 3.5 MB on the test catalog, most of it `pg_get_indexdef`. The index and foreign-key queries also returned partitions' rows, which the client then dropped. | Many restored tabs, big catalogs | **Done.** One read per connection at a time, in the window and in Rust; only the tab on screen asks; partitions' indexes and keys are skipped. |
| Medium | **No `lock_timeout` in writable modes.** DDL typed in Auto-commit or Manual waited for its lock until the statement time limit, and every reader and writer of the table queued behind it. | Production | **Done.** `lock_timeout = 5s` in writable modes, reset on a return to Read only. |
| Medium | **Auto-commit tries every write read-only first.** Each write costs about 3 extra round trips. It also leaves `cannot execute … in a read-only transaction`, with the statement text, in the server log, and counts a rollback in `xact_rollback` that Health and hosted dashboards then show. A `SELECT` that calls a writing function runs twice. | Every Auto-commit write | **Open.** Send statements that obviously write (`INSERT`, `UPDATE`, `DELETE`, `MERGE`, DDL) straight to the writable path, and keep read-only-first for ambiguous ones. |
| Medium-low | **Health:** <br>• Any error dropped the session, including its own 2 s timeout, so on a struggling server it reconnected every 10 s. <br>• A wrong password was retried every 10 s. <br>• It kept sampling while the window was minimized. <br>• Each sample took 10–12 round trips: measured 265 ms, against 27 ms prepared and pipelined. <br>• `pg_blocking_pids` ran for every backend, and it locks the lock manager's shared state. | Health open on a busy or remote server | **Done.** <br>• Errors from the server keep the session; a sample with no answer in 10 s drops it. <br>• A refused or unreadable password waits a minute (done with secrets). <br>• Sampling pauses while the window is hidden. <br>• Statements are prepared once per session and pipelined. <br>• `pg_blocking_pids` runs only for sessions waiting on a lock. |
| Medium (SQLite) | **A Manual transaction on SQLite is held until the user ends it.** No server ends it. In rollback-journal mode a read keeps other programs from committing; in WAL mode the WAL cannot be reset; a write holds the write lock. | A file another program writes to | **Open.** Roll back idle SQLite Manual transactions after a few minutes and tell the tab. |
| Low-medium | **Dead connections lingered.** After sleep or a network change the server kept the backend until its own keepalive gave up, often after 2 hours or more. On quit, sockets closed without a `Terminate` message, which logs `unexpected EOF`. | Laptops, small connection limits | **Done.** <br>• Keepalives on both ends. <br>• `idle_session_timeout` and `client_connection_check_interval` on PostgreSQL 14 and later. <br>• Quitting closes idle sessions with `Terminate`. |
| Low | **The byte budget does not bound transfer.** Each value counts at most 64 KB toward the 128 MB budget, but every batch of 1,000 rows arrives whole: 1,000 values of 10 MB each are detoasted and sent before the check runs. | Wide `jsonb` or `bytea` columns | **Open.** Size batches from the bytes already seen, starting small. |
| Low | **Export keeps a snapshot open for its whole run.** It reads in one read-only transaction with no overall limit, so a long export of a large production table holds back VACUUM for as long as it runs. | Large exports on Production | **Open.** Warn or cap the duration on Production. |
| Low | **Smaller items:** <br>• A client-side timeout during a multi-batch fetch marked the session broken but kept its socket until the next run. <br>• Explain Analyze in Read only executes the whole query, where a Run stops at the cap. <br>• `application_name` does not tell tabs apart. | — | **The first is done:** a broken session is dropped at once. The other two are open. |

### Next steps

- **Cursor reads in Manual.** This is the largest remaining cost.
- **Classifying statements** so Auto-commit writes skip the failing read-only attempt.
- **Rolling back idle SQLite Manual transactions.**
- **Batches sized by bytes**, and limits on long exports.
- **Pipelining `BEGIN READ ONLY` with Parse** would save one more round trip per run: the review measured 78 ms against 135 ms at 25 ms per round trip, together with the commit change. `tokio-postgres` offers portals only on a `Transaction`, which it creates after `BEGIN` answers, so this needs a change upstream or a different way to read a portal.

## Not in this design

- **Editing rows in the grid.** Turning cell edits into `UPDATE`s needs primary keys, conflict rules, and its own review; SQL does the job until it is clearly missed.
- **SSH tunnels.** Production databases are often behind a bastion. Until this is designed (an `ssh -L` the app runs, or `russh`), a tunnel opened in Terminal works with a connection to `localhost`. Cloud proxies such as the Cloud SQL Auth Proxy work the same way.
- **Other databases.** MySQL and others fit behind the same driver trait later.
- **Agent tools.** Querying databases through MCP is attractive and risky: results are data from production, and statements are code. A later design could offer one read-only tool for connections the user marks "agents may query"; nothing in this design exposes databases to agents.
- **AI-written SQL.** Belongs to v0.9 or later, with the schema as context.
- **Monitoring beyond the hour on screen.** Health keeps no history on disk, sends no alerts, and does not sample in the background. Cloud Monitoring and Coolify already do this, and a second copy would be one more thing to trust.
- ER diagrams, schema diffs and migrations, charts of results, CSV import, IAM and Kerberos authentication, `.pgpass` and `pg_service.conf`.

## Open questions

| Question | Why it matters | Needed before |
| --- | --- | --- |
| Are SSH tunnels needed from the start? | If the work databases are only reachable through a bastion, the client is unusable for them without one (a tunnel opened in Terminal, or the Cloud SQL Auth Proxy, works meanwhile) | v0.4 exit gate |
| Which hosted PostgreSQL providers must work (RDS, Cloud SQL, Supabase, Neon)? | The spike verified TLS against a local CA only; Neon also needs SNI, which rustls sends for a host name | v0.4 exit gate |
| Should saved queries also be files, for Git and `psql -f`? | Option C of Where saved queries live, as an export or a sync | v0.4.x |
| A fixed row cap or a time and size budget? | 1,000 rows of wide JSON is heavier than 100,000 narrow rows; cells are cut at 64 KB meanwhile | v0.4.x |
| Can `pg_monitor` be granted on Cloud SQL, and is `pg_stat_statements` on there by default? | Without them Health shows sessions without statements and no slowest queries | v0.4 exit gate |
| Application Default Credentials only, or also a service account key in the Keychain? | ADC needs `gcloud` on the Mac; a long-lived key is what Google advises against. Only `authorized_user` credentials are read | v0.4.x |
| Does Coolify's own API expose Sentinel's metrics? | It would avoid SSH for Coolify servers | v0.4.x |

## Sources

- [tokio-postgres](https://docs.rs/tokio-postgres): portals, `CancelToken`, `FromSql`.
- [PostgreSQL: extended query protocol](https://www.postgresql.org/docs/current/protocol-flow.html#PROTOCOL-FLOW-EXT-QUERY): portals and the row limit of Execute.
- [PostgreSQL: SET TRANSACTION](https://www.postgresql.org/docs/current/sql-set-transaction.html) and [client connection defaults](https://www.postgresql.org/docs/current/runtime-config-client.html): read-only transactions, `statement_timeout`, `idle_in_transaction_session_timeout`.
- [SQLite: opening a database](https://www.sqlite.org/c3ref/open.html), [RETURNING](https://www.sqlite.org/lang_returning.html), [interrupt](https://www.sqlite.org/c3ref/interrupt.html), and [the authorizer](https://www.sqlite.org/c3ref/set_authorizer.html).
- [@codemirror/lang-sql](https://github.com/codemirror/lang-sql): dialects, schema completion, and the syntax tree.
- [PostgreSQL: the cumulative statistics system](https://www.postgresql.org/docs/current/monitoring-stats.html) and [pg_stat_statements](https://www.postgresql.org/docs/current/pgstatstatements.html).
- [Cloud SQL metrics](https://cloud.google.com/sql/docs/postgres/admin-api/metrics) and [Application Default Credentials](https://cloud.google.com/docs/authentication/application-default-credentials).
- [Docker Engine API](https://docs.docker.com/reference/api/engine/) (container stats).
- [Coolify Sentinel](https://coolify.io/docs/core/observability/monitoring/sentinel).
