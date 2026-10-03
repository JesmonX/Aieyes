// Called by actions/github-script after all platform assets have been validated.
module.exports = async function publishRelease({github, context, tag = process.env.RELEASE_TAG, directory = 'dist/release'}) {
const fs = require('fs');
const path = require('path');
const repo = context.repo;
let release;
try {
  release = (await github.rest.repos.getReleaseByTag({...repo, tag})).data;
  if (!release.draft) throw new Error(`Release ${tag} is already public; use a new version.`);
} catch (error) {
  if (error.status !== 404) throw error;
}
const notes = (await github.rest.repos.generateReleaseNotes({...repo, tag_name: tag})).data.body;
const body = `macOS 14+: choose x64 for Intel, arm64 for Apple Silicon. Windows: x64 NSIS installer. Linux: x64 DEB / AppImage.\n\nmacOS uses ad-hoc signing; Windows is unsigned. SHA256SUMS covers every installer.\n\n${notes}`;
if (!release) release = (await github.rest.repos.createRelease({...repo, tag_name: tag, name: `Aieyes ${tag}`, body, draft: true, prerelease: false})).data;
const release_id = release.id;
const old = await github.paginate(github.rest.repos.listReleaseAssets, {...repo, release_id});
for (const asset of old) await github.rest.repos.deleteReleaseAsset({...repo, asset_id: asset.id});
const names = fs.readdirSync(directory).sort();
for (const name of names) {
  const data = fs.readFileSync(path.join(directory, name));
  await github.rest.repos.uploadReleaseAsset({...repo, release_id, name, data, headers: {'content-type': 'application/octet-stream', 'content-length': data.length}});
}
const uploaded = await github.paginate(github.rest.repos.listReleaseAssets, {...repo, release_id});
if (uploaded.length !== names.length || uploaded.some(a => a.state !== 'uploaded' || !names.includes(a.name) || a.size !== fs.statSync(path.join(directory, a.name)).size)) throw new Error('Release asset verification failed; draft retained.');
await github.rest.repos.updateRelease({...repo, release_id, body, draft: false, make_latest: 'true'});
};
