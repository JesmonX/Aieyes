async function discardEditor(page) {
  await page.locator('#cancel-editor').click();
  if(await page.locator('#editor-confirm-discard').isVisible())await page.locator('#editor-confirm-discard').click();
}
async function commitSettings(page) {
  await page.evaluate(()=>document.activeElement?.blur());
  await page.waitForFunction(()=>!document.querySelector('#editor')?.open&&!state.busy&&!state.generalSavePending&&!state.settingsSaving&&!state.settingsCommitting);
}
// The older fixture servers flattened URLs to basenames. Font tests must load
// the same nested, local assets as a packaged WebView instead of silently falling back.
function serveFont(req,res) {
  const pathname=new URL(req.url,'http://localhost').pathname;
  if(!pathname.startsWith('/fonts/'))return false;
  const name=pathname.slice('/fonts/'.length);
  if(!/^(HarmonyOS_Sans_SC_(Regular|Medium|Bold)\.ttf|LICENSE\.txt|sources\.json)$/.test(name)){
    res.writeHead(404);res.end();return true;
  }
  const fs=require('node:fs'),path=require('node:path');
  const file=path.join(__dirname,'../apps/desktop/web/fonts',name);
  if(!fs.existsSync(file)){res.writeHead(404);res.end();return true;}
  res.setHeader('Content-Type',name.endsWith('.ttf')?'font/ttf':name.endsWith('.json')?'application/json':'text/plain; charset=utf-8');
  res.end(fs.readFileSync(file));return true;
}
async function openGeneralPage(page, section) {
  if (!await page.locator('[data-general-page=' + section + ']').count()) await page.locator('[data-settings-tab=general]').click();
  await page.locator('[data-general-page=' + section + ']').click();
}
async function revealControl(page, selector) {
  const closed = page.locator(selector).locator('xpath=ancestor::details[not(@open)]');
  while (await closed.count()) {
    const outermost = page.locator(selector).locator('xpath=ancestor::details[not(@open)][last()]');
    await outermost.locator(':scope > summary').click();
  }
}
async function openGuide(page) {
  const summary = page.locator('.workspace-help > summary');
  if (!await summary.evaluate(el => el.parentElement.open)) await summary.click();
  await page.locator('aside [data-guide-open]').click();
}
module.exports={discardEditor,commitSettings,serveFont,openGeneralPage,revealControl,openGuide};
