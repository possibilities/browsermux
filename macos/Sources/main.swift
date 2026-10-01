import AppKit
let fm=FileManager.default
let smoke=CommandLine.arguments.contains("--smoke-test")
let root:String
if let isolated=ProcessInfo.processInfo.environment["SPB_DATA_ROOT"] {root=isolated}
else {root=fm.urls(for:.applicationSupportDirectory,in:.userDomainMask)[0].appendingPathComponent("browsermux").path}
do {
    try fm.createDirectory(atPath:root,withIntermediateDirectories:true,attributes:[.posixPermissions:0o700])
    // Open the exclusive Rust profile lock BEFORE initializing CEF.
    let app=SPBApplication.shared
    app.setActivationPolicy(.regular)
    let controller=try BrowserController(root:root)
    guard SPBInitialize(root) else {fputs("Sandbox-enabled CEF initialization failed.\n",stderr);exit(70)}
    controller.smokeTest=smoke
    app.finishLaunching()
    DispatchQueue.main.async{controller.start()}
    SPBRunLoop()
    SPBShutdown()
    withExtendedLifetime(controller){}
} catch {
    let alert=NSAlert();alert.messageText="Could not open browsermux";alert.informativeText="\(error.localizedDescription)\nExisting profile data has been preserved. Close other instances or select a fresh data root.";alert.runModal();exit(1)
}
