async function discardEditor(page) {
  await page.locator('#cancel-editor').click();
  if(await page.locator('#editor-confirm-discard').isVisible())await page.locator('#editor-confirm-discard').click();
}
async function commitSettings(page) {
  await page.waitForFunction(()=>!document.querySelector('#editor')?.open&&!state.busy);
  if(await page.locator('#settings-save-all').isEnabled()){
    await page.locator('#settings-save-all').click();
    await page.waitForFunction(()=>!state.busy&&!state.settingsCommitting);
  }
}
module.exports={discardEditor,commitSettings};
