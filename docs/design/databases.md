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

## Not in this design

- **Editing rows in the grid.** Turning cell edits into `UPDATE`s needs primary keys, conflict rules, and its own review; SQL does the job until it is clearly missed.
- **SSH tunnels.** Production databases are often behind a bastion. Until this is designed (an `ssh -L` the app runs, or `russh`), a tunnel opened in Terminal works with a connection to `localhost`. Cloud proxies such as the Cloud SQL Auth Proxy work the same way.
- **Other databases.** MySQL and others fit behind the same driver trait later.
- **Agent tools.** Querying databases through MCP is attractive and risky: results are data from production, and statements are code. A later design could offer one read-only tool for connections the user marks "agents may query"; nothing in this design exposes databases to agents.
- **AI-written SQL.** Belongs to v0.7 or later, with the schema as context.
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
