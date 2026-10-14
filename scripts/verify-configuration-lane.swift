import Foundation

@main struct VerifyConfigurationLane {
    @MainActor static func main() async throws {
        let directory = FileManager.default.temporaryDirectory.appendingPathComponent("aieyes-config-lane-" + UUID().uuidString)
        try FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: directory) }
        let script = directory.appendingPathComponent("core.py")
        let program = """
        #!/usr/bin/env python3
        import json,sys,time,os,threading
        root=os.path.dirname(__file__)
        def pet(key):
            return dict(id=key,libraryId=key,displayName=key,contentHash=key,spritePath='/fixture/pet.png',spriteVersionNumber=1,rows=9,cellWidth=128,cellHeight=144)
        if not os.path.exists(root+'/library.json'):
            json.dump(dict(pets=[pet('original')],selectedLibraryId='original',warnings=[]),open(root+'/library.json','w'))
        def held(name):
            if not os.path.exists(root+'/hold-'+name): return
            open(root+'/'+name+'-started','w').close()
            deadline=time.monotonic()+10
            while not os.path.exists(root+'/release-'+name) and time.monotonic()<deadline: time.sleep(.01)
        def answer(request):
            method=request['method']
            result={}
            if method=='fixture.block':
                open(root+'/blocked','w').close()
                time.sleep(20)
            elif method=='pets.list':
                result=json.load(open(root+'/library.json'))
                held('library')
            elif method=='pets.import':
                held('import')
                new=pet(request['params']['input'])
                library=json.load(open(root+'/library.json'))
                library['pets'].append(new); library['selectedLibraryId']=new['libraryId']
                json.dump(library,open(root+'/library.json','w'))
                result=dict(pet=new)
            elif method=='pets.select':
                library=json.load(open(root+'/library.json'))
                result=dict(pet=next(p for p in library['pets'] if p['libraryId']==request['params']['libraryId']))
            elif method=='settings.patch': result=request.get('params',{}).get('settings',{})
            elif method=='accounts.status.get' and os.path.exists(root+'/pet-model'): result=[]
            print(json.dumps({'id':request['id'],'result':result}),flush=True)
        for line in sys.stdin:
            request=json.loads(line)
            if request['method'] in ['pets.list','pets.import']:
                threading.Thread(target=answer,args=(request,),daemon=True).start()
            else: answer(request)
        """
        try program.write(to: script, atomically: true, encoding: .utf8)
        try FileManager.default.setAttributes([.posixPermissions:0o700], ofItemAtPath:script.path)
        setenv("AIEYES_CORE_PATH", script.path, 1)
        let engine = EngineClient()
        defer { engine.stop() }
        let _: Acknowledgement = try await engine.call("settings.get")
        let blocked = Task { let _: Acknowledgement? = try? await engine.call("fixture.block") }
        while !FileManager.default.fileExists(atPath:directory.appendingPathComponent("blocked").path) { try await Task.sleep(for:.milliseconds(10)) }
        for method in ["agents.set", "settings.patch", "accounts.connect", "accounts.status.get", "dashboard", "dashboard.summary", "quotas.schedule"] {
            let start = Date()
            let _: Acknowledgement = try await engine.call(method)
            precondition(Date().timeIntervalSince(start) < 0.5, "Configuration queued behind a 20-second request: " + method)
        }
        engine.stop(); await blocked.value
        print("Configuration and dashboard requests returned under 500 ms while the main channel was blocked by a 20-second operation")

        setenv("AIEYES_DATA_DIR", directory.path, 1)
        let model = AppModel(autostart: false)
        defer { model.stop() }
        func mark(_ name: String) throws { try Data().write(to: directory.appendingPathComponent(name)) }
        func exists(_ name: String) -> Bool { FileManager.default.fileExists(atPath: directory.appendingPathComponent(name).path) }
        func waitFor(_ condition: () -> Bool) async throws {
            let deadline = ProcessInfo.processInfo.systemUptime + 5
            while !condition() && ProcessInfo.processInfo.systemUptime < deadline { try await Task.sleep(for: .milliseconds(10)) }
            precondition(condition(), "Fixture did not complete within five seconds")
        }
        try mark("pet-model")
        try mark("hold-library")
        let initialLoad = Task { await model.loadPetLibrary() }
        try await waitFor { exists("library-started") }
        let importing = Task { await model.importPet(input: "new-pet") }
        try await waitFor { model.importedPet?.libraryKey == "new-pet" }
        try mark("release-library")
        await importing.value; await initialLoad.value
        precondition(model.petLibrary.map(\.libraryKey) == ["original", "new-pet"] && model.importedPet?.libraryKey == "new-pet" && !model.petLibraryBusy && !model.petImportBusy, "An older library response must not hide a successfully imported pet")
        try mark("hold-import")
        let downloading = Task { await model.importPet(input: "downloaded-pet") }
        try await waitFor { exists("import-started") }
        model.settingsDraft.refreshSeconds = 777
        await model.saveGeneralConfiguration()
        precondition(model.settings.refreshSeconds == 777 && !model.settingsSaving && model.petImportBusy, "Slow pet downloads must not block configuration saves")
        try mark("release-import")
        await downloading.value
        precondition(model.petImportError == nil && model.petLibrary.count == 3 && model.importedPet?.libraryKey == "downloaded-pet")
        await model.selectPet(model.petLibrary.first!)
        precondition(model.importedPet?.libraryKey == "original" && model.petLibrary.count == 3 && model.petSelectionBusy == nil)
        print("Native pet library: stale metadata response recovery, import selection, retained entries, and configuration saves during slow downloads passed")
    }
}
