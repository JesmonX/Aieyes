import test from 'node:test';
import assert from 'node:assert/strict';
import baseline from './release-ci-baseline.cjs';

const bump = `diff --git a/Cargo.lock b/Cargo.lock
index 1..2 100644
--- a/Cargo.lock
+++ b/Cargo.lock
@@ -10 +10 @@
-version = "0.1.63"
+version = "0.1.64"
diff --git a/apps/desktop/src-tauri/tauri.conf.json b/apps/desktop/src-tauri/tauri.conf.json
--- a/apps/desktop/src-tauri/tauri.conf.json
+++ b/apps/desktop/src-tauri/tauri.conf.json
@@ -3 +3 @@
-  "version": "0.1.63",
+  "version": "0.1.64",
diff --git a/apps/macos/Info.plist b/apps/macos/Info.plist
--- a/apps/macos/Info.plist
+++ b/apps/macos/Info.plist
@@ -20,2 +20,2 @@
-<key>CFBundleVersion</key><string>64</string>
+<key>CFBundleVersion</key><string>65</string>
diff --git a/docs/release-notes/v0.1.64.md b/docs/release-notes/v0.1.64.md
new file mode 100644
--- /dev/null
+++ b/docs/release-notes/v0.1.64.md
@@ -0,0 +1 @@
+- Fixed things
`;

function fixture(passed, diff = bump) {
  const github = {rest: {actions: {async listWorkflowRuns({workflow_id, head_sha}) {
    return {data: {total_count: passed[head_sha]?.includes(workflow_id) ? 1 : 0}};
  }}}};
  const git = args => args[0] === 'rev-list' ? 'tag parent\n' : diff;
  return baseline.releaseBaseline({github, context: {repo: {owner: 'test', repo: 'aieyes'}}, sha: 'tag', git});
}
const both = ['core-native.yml', 'desktop.yml'];

test('version bumps and release notes reuse green parent CI', async () => {
  assert.equal((await fixture({parent: both})).fullTests, false);
});
test('green CI on the tagged commit itself is reused', async () => {
  assert.equal((await fixture({tag: both}, 'diff --git a/crates/core/src/lib.rs b/crates/core/src/lib.rs\n+fix\n')).fullTests, false);
});
test('missing or partial parent CI runs full release tests', async () => {
  assert.equal((await fixture({})).fullTests, true);
  assert.equal((await fixture({parent: ['core-native.yml']})).fullTests, true);
});
test('code or dependency changes in the release commit run full release tests', async () => {
  assert.equal((await fixture({parent: both}, bump + 'diff --git a/crates/core/src/lib.rs b/crates/core/src/lib.rs\n+fix\n')).fullTests, true);
  assert.equal((await fixture({parent: both}, bump.replace('+version = "0.1.64"', '+version = "0.1.64"\n+checksum = "abc"'))).fullTests, true);
  assert.equal(baseline.releaseOnlyChange('diff --git a/apps/macos/Info.plist b/apps/macos/Info.plist\n+<key>LSUIElement</key><true/>\n'), false);
});
