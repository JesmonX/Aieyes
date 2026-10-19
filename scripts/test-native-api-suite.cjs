/* Exercise every native API form with the real core and isolated local fixtures. */
const {spawnSync}=require('node:child_process');
const path=require('node:path');
const cases=[['opencode'],['codebuddy'],['kimi'],['deepseek-harness'],['claude','api-key'],['claude','auth-token'],['zcode'],['grok'],['qoder'],['pi'],['omp']];
for(const [provider,mode='api-key'] of cases){
  const result=spawnSync(process.execPath,[path.join(__dirname,'test-native-api-ui.cjs')],{
    stdio:'inherit',env:{...process.env,AIEYES_API_UI_PROVIDER:provider,AIEYES_API_UI_AUTH_MODE:mode},
  });
  if(result.error){console.error(result.error);process.exit(1);}
  if(result.status!==0)process.exit(result.status||1);
}
console.log('Native API suite: ten providers and both Claude authentication modes passed');
