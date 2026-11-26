Responses captured from OpenCode v2.1.0 (`opencode serve` in an isolated HOME / XDG tree, a
scratch project and a local test model). Each file is the unmodified body of
`GET /api/session/{id}/permission` or `GET /api/session/{id}/form`.

- `permission-shell.json`: `echo probe > out.txt && ls -la` through the shell tool.
- `permission-edit.json`: an edit of `a.txt`, with the patch in `metadata.files`.
- `permissions-parallel.json`: two shell tool calls issued in one assistant message.
- `form-question.json`: the question tool asking a single-choice and a multiple-choice question.
- `form-fields.json`: a form created through `POST /api/session/{id}/form` with every field type.
