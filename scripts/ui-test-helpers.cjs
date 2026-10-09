async function discardEditor(page) {
  await page.locator('#cancel-editor').click();
  if(await page.locator('#editor-confirm-discard').isVisible())await page.locator('#editor-confirm-discard').click();
}
async function commitSettings(page) {
  await page.evaluate(()=>document.activeElement?.blur());
  await page.waitForFunction(()=>!document.querySelector('#editor')?.open&&!state.busy&&!state.generalSavePending&&!state.settingsSaving&&!state.settingsCommitting);
}
module.exports={discardEditor,commitSettings};
