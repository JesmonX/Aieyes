// Keep the licensed font files byte-for-byte identical to their pinned sources.
const assert=require('node:assert/strict');
const fs=require('node:fs'),path=require('node:path'),crypto=require('node:crypto');
function checkBundledFonts() {
  const root=path.join(__dirname,'../apps/desktop/web/fonts');
  const manifest=JSON.parse(fs.readFileSync(path.join(root,'sources.json'),'utf8'));
  assert.match(manifest.revision,/^[a-f0-9]{40}$/);
  for(const font of manifest.fonts) {
    assert.equal(path.basename(font.file),font.file);
    const bytes=fs.readFileSync(path.join(root,font.file));
    assert.equal(bytes.length,font.bytes,`${font.file}: original file size`);
    assert.equal(crypto.createHash('sha256').update(bytes).digest('hex'),font.sha256,`${font.file}: original font hash`);
    assert(font.source.includes('/'+manifest.revision+'/'),'font source must use the pinned revision');
  }
  assert.match(fs.readFileSync(path.join(root,'LICENSE.txt'),'utf8'),/HarmonyOS Sans Fonts License Agreement/);
  return manifest.fonts.length;
}
module.exports=checkBundledFonts;
if(require.main===module)console.log(`Bundled fonts: ${checkBundledFonts()} original HarmonyOS faces and license verified`);
