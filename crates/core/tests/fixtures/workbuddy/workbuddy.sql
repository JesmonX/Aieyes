-- Synthetic WorkBuddy database. Tables and columns are those of WorkBuddy 5.5.6
-- (macOS, ~/.workbuddy/workbuddy.db); every row is made up. Timestamps are
-- relative to the moment the script runs, so it is applied at test time:
-- Rust tests (include_str!), scripts/test-native-database-remote.py,
-- scripts/test-account-scan.py and scripts/test-native-credits-ui.cjs.
CREATE TABLE sessions (
    id TEXT PRIMARY KEY,
    cwd TEXT NOT NULL,
    user_id TEXT NOT NULL,
    title TEXT,
    custom_title TEXT,
    status TEXT NOT NULL DEFAULT 'Pending',
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    last_activity_at INTEGER,
    deleted_at INTEGER,
    is_playground INTEGER NOT NULL DEFAULT 0,
    source_mode TEXT,
    is_background_automation INTEGER,
    mode TEXT,
    model TEXT,
    expert_id TEXT,
    expert_locale TEXT,
    expert_runtime_identity TEXT,
    expert_marketplace TEXT,
    permission_mode TEXT,
    use_sandbox_cli INTEGER,
    project_id TEXT
, plugin_context_json TEXT, addon_selection TEXT, session_settings TEXT, last_user_prompt_expert_selection TEXT, context_window INTEGER, buddy_snapshot_id TEXT, buddy_binding_json TEXT, thought_level TEXT);
CREATE TABLE automation_runs (
    thread_id TEXT PRIMARY KEY,
    automation_id TEXT NOT NULL,
    status TEXT NOT NULL,
    read_at INTEGER,
    thread_title TEXT,
    source_cwd TEXT,
    runs_json TEXT,
    result_success INTEGER,
    metadata_json TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
CREATE TABLE session_usage (
    session_id TEXT PRIMARY KEY,
    used INTEGER NOT NULL,
    size INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    credit_json TEXT
);
-- id, user, status, activity (ms), deleted, playground, source_mode, background, model
INSERT INTO sessions(id,cwd,user_id,title,status,created_at,updated_at,last_activity_at,deleted_at,is_playground,source_mode,is_background_automation,mode,model) VALUES
 ('user-chat','/fixture','fixture-user','PRIVATE_TITLE','completed',CAST(strftime('%s','now') AS INTEGER)*1000-120000,CAST(strftime('%s','now') AS INTEGER)*1000-60000,CAST(strftime('%s','now') AS INTEGER)*1000-60000,NULL,0,'craft',NULL,'craft','fixture-model'),
 ('user-live','/fixture','fixture-user','PRIVATE_TITLE','working',CAST(strftime('%s','now') AS INTEGER)*1000-90000,CAST(strftime('%s','now') AS INTEGER)*1000-30000,CAST(strftime('%s','now') AS INTEGER)*1000-30000,NULL,0,'craft',NULL,'craft','fixture-model'),
 ('user-waiting','/fixture','fixture-user','PRIVATE_TITLE','waiting_permission',CAST(strftime('%s','now') AS INTEGER)*1000-80000,CAST(strftime('%s','now') AS INTEGER)*1000-20000,CAST(strftime('%s','now') AS INTEGER)*1000-20000,NULL,0,'craft',NULL,'craft','fixture-model'),
 ('background','/fixture','fixture-user','PRIVATE_TITLE','working',CAST(strftime('%s','now') AS INTEGER)*1000-70000,CAST(strftime('%s','now') AS INTEGER)*1000-10000,CAST(strftime('%s','now') AS INTEGER)*1000-10000,NULL,0,NULL,1,'craft','fixture-model'),
 ('background-mode','/fixture','fixture-user','PRIVATE_TITLE','working',CAST(strftime('%s','now') AS INTEGER)*1000-70000,CAST(strftime('%s','now') AS INTEGER)*1000-10000,CAST(strftime('%s','now') AS INTEGER)*1000-10000,NULL,0,'background',NULL,'craft','fixture-model'),
 ('playground','/fixture','fixture-user','PRIVATE_TITLE','working',CAST(strftime('%s','now') AS INTEGER)*1000-70000,CAST(strftime('%s','now') AS INTEGER)*1000-10000,CAST(strftime('%s','now') AS INTEGER)*1000-10000,NULL,1,'craft',NULL,'craft','fixture-model'),
 ('automation-thread','/fixture','fixture-user','PRIVATE_TITLE','working',CAST(strftime('%s','now') AS INTEGER)*1000-70000,CAST(strftime('%s','now') AS INTEGER)*1000-10000,CAST(strftime('%s','now') AS INTEGER)*1000-10000,NULL,0,'craft',NULL,'craft','fixture-model'),
 ('deleted','/fixture','fixture-user','PRIVATE_TITLE','working',CAST(strftime('%s','now') AS INTEGER)*1000-70000,CAST(strftime('%s','now') AS INTEGER)*1000-10000,CAST(strftime('%s','now') AS INTEGER)*1000-10000,CAST(strftime('%s','now') AS INTEGER)*1000-5000,0,'craft',NULL,'craft','fixture-model'),
 ('earlier-user','/fixture','fixture-earlier-user','PRIVATE_TITLE','completed',CAST(strftime('%s','now') AS INTEGER)*1000-70000,CAST(strftime('%s','now') AS INTEGER)*1000-50000,CAST(strftime('%s','now') AS INTEGER)*1000-50000,NULL,0,'craft',NULL,'craft','other-model'),
 ('old','/fixture','fixture-user','PRIVATE_TITLE','completed',CAST(strftime('%s','now','start of month','-1 month') AS INTEGER)*1000,CAST(strftime('%s','now','start of month','-1 month') AS INTEGER)*1000,CAST(strftime('%s','now','start of month','-1 month') AS INTEGER)*1000,NULL,0,'craft',NULL,'craft','fixture-model');
INSERT INTO automation_runs(thread_id,automation_id,status,created_at,updated_at) VALUES ('automation-thread','fixture-automation','completed',1,1);
-- credit_json as written by persistUsage: {requestId: credits}; NULL on free requests.
INSERT INTO session_usage VALUES
 ('user-chat',50664,300000,CAST(strftime('%s','now') AS INTEGER)*1000-60000,'{"req-1":0.25,"req-2":1.5}'),
 ('user-live',1200,300000,CAST(strftime('%s','now') AS INTEGER)*1000-30000,'{"req-3":0.5}'),
 ('background',800,300000,CAST(strftime('%s','now') AS INTEGER)*1000-10000,'{"req-4":2}'),
 ('playground',400,300000,CAST(strftime('%s','now') AS INTEGER)*1000-10000,NULL),
 ('earlier-user',900,300000,CAST(strftime('%s','now') AS INTEGER)*1000-50000,'{"req-5":0.125}'),
 ('old',100,300000,CAST(strftime('%s','now','start of month','-1 month') AS INTEGER)*1000,'{"req-old":100}');
