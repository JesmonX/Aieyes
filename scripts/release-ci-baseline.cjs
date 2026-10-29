// Called by actions/github-script before release builds: reuse branch CI when it already verified the tagged source.
const {execFileSync} = require('child_process');

const REQUIRED_WORKFLOWS = ['core-native.yml', 'desktop.yml'];
// Release preparation may only bump these version fields and add release notes.
const VERSION_FILES = new Set(['Cargo.lock', 'crates/core/Cargo.toml', 'apps/desktop/src-tauri/Cargo.toml', 'apps/desktop/src-tauri/Cargo.lock', 'apps/desktop/src-tauri/tauri.conf.json', 'apps/macos/Info.plist']);
const VERSION_LINE = [/^version = "[^"]+"$/, /^"version": "[^"]+",?$/, /^(<key>CFBundle(ShortVersionString|Version)<\/key>)?<string>[0-9.]+<\/string>$/];

function releaseOnlyChange(diff) {
  let file = null;
  for (const line of diff.split('\n')) {
    if (line.startsWith('diff --git ')) {
      file = line.split(' b/')[1];
      if (!/^docs\/release-notes\/[^/]+\.md$/.test(file) && !VERSION_FILES.has(file)) return false;
    } else if (/^(new|deleted) file mode|^rename |^Binary files /.test(line)) {
      if (!file.startsWith('docs/release-notes/')) return false;
    } else if (/^[+-]/.test(line) && !/^(\+\+\+|---) /.test(line) && VERSION_FILES.has(file)) {
      if (!VERSION_LINE.some(pattern => pattern.test(line.slice(1).trim()))) return false;
    }
  }
  return true;
}

async function verified({github, repo, sha}) {
  for (const workflow_id of REQUIRED_WORKFLOWS) {
    const runs = (await github.rest.actions.listWorkflowRuns({...repo, workflow_id, head_sha: sha, status: 'success', per_page: 1})).data;
    if (!runs.total_count) return false;
  }
  return true;
}

async function releaseBaseline({github, context, sha, git = args => execFileSync('git', args, {encoding: 'utf8', maxBuffer: 64 << 20})}) {
  const repo = context.repo;
  if (await verified({github, repo, sha})) return {fullTests: false, reason: `branch CI passed on ${sha}`};
  const parents = git(['rev-list', '--parents', '-n', '1', sha]).trim().split(' ').slice(1);
  if (parents.length !== 1) return {fullTests: true, reason: 'release commit is not a single-parent commit'};
  const [parent] = parents;
  if (!releaseOnlyChange(git(['diff', '-U0', parent, sha]))) return {fullTests: true, reason: 'release commit changes more than versions and release notes'};
  if (await verified({github, repo, sha: parent})) return {fullTests: false, reason: `branch CI passed on parent ${parent}`};
  return {fullTests: true, reason: `${REQUIRED_WORKFLOWS.join(' and ')} have not both passed on ${parent}`};
}

module.exports = async function ({github, context, core, force = false}) {
  const sha = execFileSync('git', ['rev-parse', 'HEAD'], {encoding: 'utf8'}).trim();
  const {fullTests, reason} = force ? {fullTests: true, reason: 'requested by workflow input'} : await releaseBaseline({github, context, sha});
  core.setOutput('full_tests', String(fullTests));
  await core.summary.addRaw(`${fullTests ? 'Running full release tests' : 'Reusing branch CI'}: ${reason}\n`).write();
};
module.exports.releaseOnlyChange = releaseOnlyChange;
module.exports.releaseBaseline = releaseBaseline;
