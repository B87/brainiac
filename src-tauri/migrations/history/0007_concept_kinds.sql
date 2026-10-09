-- v0.6.1: a concept's kind `system` became `tool` (migration 0012 of
-- brainiac.db). Explanations are stored as the JSON the app wrote, compact,
-- so a plain replace finds each concept's kind; a note's text that held the
-- same words would have its quotes escaped, and does not match.
UPDATE explanations
SET explanation = REPLACE(explanation, '"kind":"system"', '"kind":"tool"')
WHERE explanation LIKE '%"kind":"system"%';
