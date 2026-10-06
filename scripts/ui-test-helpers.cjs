async function discardEditor(page) {
  await page.locator('#cancel-editor').click();
  if(await page.locator('#editor-confirm-discard').isVisible())await page.locator('#editor-confirm-discard').click();
}
module.exports={discardEditor};
