-- A named secret a run gets as a file (the variable holds the path), for
-- tools that read a key file, like a Google service account key.
ALTER TABLE named_secrets ADD COLUMN as_file INTEGER NOT NULL DEFAULT 0;
