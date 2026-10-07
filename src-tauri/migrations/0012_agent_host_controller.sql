-- v0.5 host jobs (SPEC.md, Remote hosts, Upgrade): the build of the run
-- controller Install put on a host, so Settings can say an upgrade is
-- available. NULL for a controller installed before this was recorded.
ALTER TABLE agent_hosts ADD COLUMN controller_build TEXT;
ALTER TABLE agent_hosts ADD COLUMN controller_installed_at TEXT;
