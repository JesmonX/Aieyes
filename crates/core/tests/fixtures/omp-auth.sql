-- Synthetic OMP 18.12.2 schema. These are fixture credentials, not user data.
CREATE TABLE auth_credentials (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    provider TEXT NOT NULL,
    credential_type TEXT NOT NULL,
    data TEXT NOT NULL,
    disabled_cause TEXT DEFAULT NULL,
    identity_key TEXT DEFAULT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
INSERT INTO auth_credentials VALUES
    (1, 'google-gemini-cli', 'oauth', '{"access":"PRIVATE-GEMINI-ONE","refresh":"PRIVATE-GEMINI-REFRESH","expires":4102444800000,"projectId":"project-one","email":"person@example.test","unknownSecret":"PRIVATE-UNRELATED-FIELD"}', NULL, 'email:person@example.test', 1, 1),
    (2, 'google-gemini-cli', 'oauth', '{"access":"PRIVATE-GEMINI-DISABLED","expires":4102444800000,"projectId":"project-two"}', 'signed out', 'email:previous@example.test', 1, 2),
    (3, 'github-copilot', 'oauth', '{"access":"PRIVATE-COPILOT-SESSION","refresh":"gho_PRIVATE-GITHUB","expires":1,"email":"fixture"}', NULL, 'email:fixture', 1, 3),
    (4, 'unrelated-provider', 'oauth', 'PRIVATE-DO-NOT-PARSE', NULL, NULL, 1, 4),
    (5, 'google-antigravity', 'oauth', '{"access":"PRIVATE-ANTIGRAVITY","projectId":"project-one","expires":4102444800000}', NULL, NULL, 1, 5),
    (6, 'google-gemini-cli', 'api_key', '{"key":"PRIVATE-GEMINI-API-KEY"}', NULL, NULL, 1, 6),
    (7, 'google-gemini-cli', 'oauth', '{"access":"PRIVATE-GEMINI-SECOND","expires":4102444800000,"projectId":"project-second"}', NULL, 'email:second@example.test', 1, 7),
    (8, 'anthropic', 'oauth', '{"access":"PRIVATE-CLAUDE-ACCESS","refresh":"PRIVATE-CLAUDE-REFRESH","expires":4102444800000}', NULL, NULL, 1, 8),
    (9, 'openai-codex', 'oauth', '{"access":"PRIVATE-CODEX-ACCESS","refresh":"PRIVATE-CODEX-REFRESH","expires":4102444800000,"accountId":"workspace-one","metadata":{"accountID":"workspace-one","unknownSecret":"PRIVATE-METADATA"}}', NULL, NULL, 1, 9);
