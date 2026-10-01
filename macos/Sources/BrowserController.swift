import AppKit
final class BrowserController:NSObject,NSWindowDelegate,NSTextFieldDelegate {
    let store:BrowserStore
    let dataRoot:String
    var state:Snapshot
    let window:NSWindow
    let canvas=PaneCanvas()
    let scroll=NSScrollView()
    let top=FlippedView()
    let address=NSTextField()
    let tabs=NSPopUpButton()
    let containers=NSPopUpButton()
    let status=NSTextField(labelWithString:"")
    var surfaces:[String:NSView]=[:]
    var pendingClose=Set<String>()
    var pendingPaneClose:String?
    var isQuitting=false
    var prefix=false
    var prefixMonitor:Any?
    var focusMonitor:Any?
    var bypassPrefix=false
    var focusMode=false
    var flushWork:DispatchWorkItem?
    let persistenceQueue=DispatchQueue(label:"browser.persistence",qos:.utility)
    var downloadActive:[String:Set<Int>]=[:]
    var loading:[String:Bool]=[:]
    var engineEpoch:[String:UInt64]=[:]
    var closedDuringQuit=Set<String>()
    var smokeTest=false
    var engineReady=false
    init(root:String)throws {
        dataRoot=root;store=try BrowserStore.open(dataRoot:root);state=try decode(Snapshot.self,store.snapshotJson())
        window=NSWindow(contentRect:NSRect(x:0,y:0,width:1280,height:820),styleMask:[.titled,.closable,.miniaturizable,.resizable,.fullSizeContentView],backing:.buffered,defer:false)
        super.init()
        window.delegate=self;window.title="browsermux";window.titleVisibility = .hidden;window.titlebarAppearsTransparent=true
        window.minSize=NSSize(width:480,height:320);window.isRestorable=false;window.center()
        guard let content=window.contentView else{return}
        content.addSubview(top);content.addSubview(scroll);content.addSubview(status)
        scroll.documentView=canvas;scroll.hasHorizontalScroller=true;scroll.hasVerticalScroller=true;scroll.autohidesScrollers=true;scroll.drawsBackground=false
        status.font = .systemFont(ofSize:11);status.textColor = .secondaryLabelColor
        let back=button("chevron.left",label:"Back",action:#selector(back))
        let forward=button("chevron.right",label:"Forward",action:#selector(forward))
        let reload=button("arrow.clockwise",label:"Reload or stop",action:#selector(reload))
        let plus=button("plus",label:"New tab",action:#selector(newTab))
        back.frame=NSRect(x:78,y:10,width:26,height:28);forward.frame=NSRect(x:104,y:10,width:26,height:28);reload.frame=NSRect(x:130,y:10,width:28,height:28)
        for b in [back,forward,reload,plus]{top.addSubview(b)}
        address.placeholderString="Enter URL or search…";address.font = .systemFont(ofSize:13);address.isBezeled=true;address.bezelStyle = .roundedBezel;address.target=self;address.action=#selector(navigateAddress);address.delegate=self;address.setAccessibilityLabel("Address for focused pane")
        tabs.target=self;tabs.action=#selector(selectTab);tabs.setAccessibilityLabel("Tabs in focused pane")
        containers.target=self;containers.action=#selector(selectContainer);containers.setAccessibilityLabel("Container for focused pane")
        for v in [address,tabs,containers]{top.addSubview(v)}
        plus.tag=99
        canvas.select={[weak self] id in self?.send("focus_pane",["pane_id":id],focus:true)}
        canvas.submit={[weak self] text in self?.navigate(text)}
        canvas.resize={[weak self] id,ratio in self?.send("resize_split",["split_id":id,"ratio":ratio])}
        SPBSetEventHandler {[weak self] event in DispatchQueue.main.async {self?.event(event)}}
        prefixMonitor=NSEvent.addLocalMonitorForEvents(matching:.keyDown){[weak self] in self?.key($0) ?? $0}
        focusMonitor=NSEvent.addLocalMonitorForEvents(matching:[.leftMouseDown,.rightMouseDown]){[weak self] event in
            guard let self=self,event.window===self.window else{return event}
            let point=self.canvas.convert(event.locationInWindow,from:nil)
            if let id=self.state.layout.first(where:{$0.visible && $0.rect.rect.contains(point)})?.paneId,id != self.state.workspace.focusedPane {self.send("focus_pane",["pane_id":id],focus:true)}
            return event
        }
        NotificationCenter.default.addObserver(self,selector:#selector(quitRequested),name:NSNotification.Name("SPBQuitRequested"),object:nil)
        DistributedNotificationCenter.default().addObserver(self,selector:#selector(appearanceChanged),name:NSNotification.Name("AppleInterfaceThemeChangedNotification"),object:nil)
        menus();layout();render();window.makeKeyAndOrderFront(nil);NSApp.activate(ignoringOtherApps:true)
    }
    func start() {engineReady=true;reconcile();appearanceChanged();if let notice=store.recoveryNotice(){showError(notice)};window.makeFirstResponder(address);if smokeTest{runSmokeTest()}}
    func button(_ symbol:String,label:String,action:Selector)->NSButton {let b=NSButton(image:NSImage(systemSymbolName:symbol,accessibilityDescription:label)!,target:self,action:action);b.bezelStyle = .texturedRounded;b.isBordered=false;b.toolTip=label;b.setAccessibilityLabel(label);return b}
    func layout() {
        guard let content=window.contentView else{return};let width=content.bounds.width,height=content.bounds.height
        let bar:CGFloat=focusMode ? 30:48
        top.frame=NSRect(x:0,y:height-bar,width:width,height:bar)
        scroll.frame=NSRect(x:0,y:24,width:width,height:max(160,height-bar-24))
        status.frame=NSRect(x:12,y:4,width:width-24,height:16)
        address.frame=NSRect(x:168,y:10,width:max(100,width-420),height:28)
        containers.frame=NSRect(x:width-240,y:10,width:154,height:28)
        tabs.frame=NSRect(x:width-85,y:10,width:42,height:28)
        top.viewWithTag(99)?.frame=NSRect(x:width-39,y:10,width:30,height:28)
        if focusMode {address.isHidden=true;tabs.isHidden=true;for v in top.subviews where v is NSButton {v.isHidden=true};containers.frame=NSRect(x:82,y:2,width:220,height:24)}
        else {address.isHidden=false;tabs.isHidden=false;for v in top.subviews where v is NSButton {v.isHidden=false}}
        send("set_viewport",["width":max(240,scroll.contentSize.width),"height":max(160,scroll.contentSize.height)])
    }
    func windowDidResize(_ notification:Notification){layout()}
    func send(_ type:String,_ fields:[String:Any]=[:],focus:Bool=false) {
        do {
            let result=try decode(ResultEnvelope.self,store.dispatchJson(command:encodeCommand(type,fields)))
            state=result.snapshot;render()
            for effect in result.effects {apply(effect)}
            reconcile()
            if focus {SPBFocus(state.focused.activeTabId)}
            if !["navigation_started","set_viewport"].contains(type) {scheduleFlush()}
        } catch {
            if smokeTest {
                fputs("NATIVE_SMOKE command failed: \(type): \(error.localizedDescription)\n",stderr)
                exit(23)
            }
            showError(error.localizedDescription)
        }
    }
    func scheduleFlush(){flushWork?.cancel();let work=DispatchWorkItem{[weak self] in guard let self=self else{return};do{try self.store.flush()}catch{DispatchQueue.main.async{self.showError("Workspace could not be saved. \(error.localizedDescription)")}}};flushWork=work;persistenceQueue.asyncAfter(deadline:.now()+0.35,execute:work)}
    func render() {
        canvas.update(state)
        let pane=state.focused,container=state.container(pane.containerId)
        if window.firstResponder !== address.currentEditor(){address.stringValue=pane.active.url=="about:blank" ? "":pane.active.url}
        tabs.removeAllItems();for tab in pane.tabs {tabs.addItem(withTitle:tab.title.isEmpty ? "New Tab":tab.title);tabs.lastItem?.representedObject=tab.id};tabs.selectItem(at:pane.tabs.firstIndex(where:{$0.id==pane.activeTabId}) ?? 0)
        containers.removeAllItems()
        for c in state.containers {containers.addItem(withTitle:"\(c.name) · \(c.persistence=="temporary" ? "Temp":String(c.id.prefix(4)))");containers.lastItem?.representedObject=c.id}
        containers.selectItem(at:state.containers.firstIndex(where:{$0.id==pane.containerId}) ?? 0)
        containers.menu?.addItem(.separator());containers.addItem(withTitle:"New persistent container…");containers.lastItem?.representedObject="new_persistent";containers.addItem(withTitle:"New temporary container…");containers.lastItem?.representedObject="new_temporary"
        window.title="\(pane.active.title.isEmpty ? "browsermux":pane.active.title) · \(container.name)"
        for p in state.panes {for tab in p.tabs {guard let surface=surfaces[tab.id],let host=canvas.panes[p.id]?.surface else{continue};if surface.superview !== host{surface.removeFromSuperview();host.addSubview(surface)};surface.frame=host.bounds;surface.isHidden=tab.id != p.activeTabId || tab.url=="about:blank"}}
    }
    func reconcile() {
        guard engineReady else { return }
        canvas.layoutSubtreeIfNeeded()
        for pane in state.panes {for tab in pane.tabs where surfaces[tab.id]==nil {
            if createSurface(tab.id,containerID:pane.containerId,paneID:pane.id),tab.url != "about:blank" {engineEpoch[tab.id]=SPBNavigate(tab.id,tab.url)}
        }}
        render()
    }
    @discardableResult func createSurface(_ tabID:String,containerID:String,paneID:String?)->Bool {
        if surfaces[tabID] != nil{return true}
        let surface=NSView(frame:NSRect(x:0,y:0,width:800,height:600));surface.autoresizingMask=[.width,.height]
        if let paneID=paneID,let host=canvas.panes[paneID]?.surface {surface.frame=host.bounds;host.addSubview(surface)} else {canvas.addSubview(surface);surface.isHidden=true}
        surfaces[tabID]=surface
        let c=state.container(containerID)
        let cache=c.persistence=="persistent" ? "\(dataRoot)/engine/profiles/\(c.id)":""
        guard SPBCreateTab(tabID,containerID,cache,surface) else {surfaces.removeValue(forKey:tabID);surface.removeFromSuperview();showError("Chromium could not create this tab. No fallback profile was used.");return false}
        return true
    }
    func apply(_ effect:Effect) {
        guard engineReady else { return }
        switch effect.type {
        case "create_tab":if let id=effect.tabId,let c=effect.containerId {let pane=state.panes.first(where:{$0.tabs.contains(where:{$0.id==id})});_ = createSurface(id,containerID:c,paneID:pane?.id)}
        case "navigate":if let id=effect.tabId,let url=effect.url {engineEpoch[id]=SPBNavigate(id,url)}
        case "close_tab":if let id=effect.tabId {SPBCloseTab(id)}
        case "retire_container":if let id=effect.containerId {SPBRetireContainer(id)}
        default:break
        }
    }
    func event(_ e:[String:Any]) {
        guard let type=e["type"] as? String,let id=e["tab_id"] as? String else{return}
        if let epoch=(e["navigation_epoch"] as? NSNumber)?.uint64Value {
            if type=="navigation_started" {
                if epoch < (engineEpoch[id] ?? 0){return}
                engineEpoch[id]=epoch
            } else if epoch != (engineEpoch[id] ?? epoch){return}
        }
        switch type {
        case "closed":
            surfaces.removeValue(forKey:id)?.removeFromSuperview()
            if isQuitting{closedDuringQuit.insert(id)}
            if !isQuitting,pendingClose.remove(id) != nil,let pane=state.panes.first(where:{$0.tabs.contains(where:{$0.id==id})}) {
                send("close_tab",["pane_id":pane.id,"tab_id":id,"confirmed":true])
                if let closing=pendingPaneClose,pendingClose.isEmpty{pendingPaneClose=nil;send("close_pane",["pane_id":closing,"confirmed":true])}
            } else if !isQuitting,state.tab(id) != nil {
                // Late close from a cancelled quit has no live page state to safely replay.
                send("navigate",["tab_id":id,"url":"about:blank"])
            }
        case "close_cancelled":
            isQuitting=false;pendingClose.remove(id);pendingPaneClose=nil
            // Already closed pages cannot be resumed or blindly replayed after another tab cancels quit.
            let wasReady=engineReady;engineReady=false
            for closed in closedDuringQuit where state.tab(closed) != nil {send("navigate",["tab_id":closed,"url":"about:blank"])}
            engineReady=wasReady;closedDuringQuit.removeAll();reconcile();showError("Close cancelled. Pages already closed during the attempt reopen blank; unsaved remaining pages stay open.")
        case "focused":
            if let pane=state.panes.first(where:{$0.tabs.contains(where:{$0.id==id})}),pane.id != state.workspace.focusedPane {send("focus_pane",["pane_id":pane.id])}
        case "navigation_started":if let tab=state.tab(id){send("navigation_started",["tab_id":id,"expected_generation":tab.navigationGeneration])}
        case "navigation_cancelled":
            if let tab=state.tab(id),let url=e["url"] as? String {send("navigation_committed",["tab_id":id,"expected_generation":tab.navigationGeneration,"url":url,"title":tab.title,"reopen_allowed":false])}
        case "address","title":
            if let tab=state.tab(id){send("navigation_committed",["tab_id":id,"expected_generation":tab.navigationGeneration,"url":e["url"] as? String ?? tab.url,"title":String((e["title"] as? String ?? tab.title).prefix(4096)),"reopen_allowed":e["reopen_allowed"] as? Bool ?? tab.reopenAllowed])}
        case "loading":loading[id]=e["loading"] as? Bool
        case "renderer_failed":if let tab=state.tab(id){send("renderer_failed",["tab_id":id,"expected_generation":tab.navigationGeneration]);showError("A page renderer stopped. Reload that tab to recover; other panes remain open.")}
        case "popup":
            guard let url=e["url"] as? String,let pane=state.panes.first(where:{$0.tabs.contains(where:{$0.id==id})}) else{return}
            send("new_tab",["pane_id":pane.id]);if let new=state.panes.first(where:{$0.id==pane.id})?.activeTabId{send("navigate",["tab_id":new,"url":url])}
        case "download":
            if let download=e["id"] as? Int {
                if e["active"] as? Bool == true {
                    downloadActive[id,default:[]].insert(download)
                } else {
                    downloadActive[id]?.remove(download)
                }
            }
            if state.tab(id) != nil {
                send("set_tab_warnings",["tab_id":id,"has_before_unload":false,"active_downloads":downloadActive[id]?.count ?? 0])
            }
            status.stringValue="Download · \(e["name"] as? String ?? "file") · \(e["percent"] as? Int ?? 0)%"
        case "permission_denied":showError(e["reason"] as? String ?? "Permission denied. Unhandled permissions are blocked by default.")
        case "blocked_navigation":showError("This navigation scheme is not allowed. Only HTTP and HTTPS pages are supported.")
        case "load_error":showError("Page load failed (\(e["code"] as? Int ?? 0)). Certificate errors remain blocked.")
        default:break
        }
    }
    func showError(_ message:String){status.stringValue=message;status.toolTip=message;NSAccessibility.post(element:status,notification:.announcementRequested,userInfo:[.announcement:message,.priority:NSAccessibilityPriorityLevel.high.rawValue])}
    func navigate(_ input:String) {
        let text=input.trimmingCharacters(in:.whitespacesAndNewlines);guard !text.isEmpty else{return}
        var candidate=text
        if !text.contains("://"),text != "about:blank" {
            if !text.contains(" "),text.contains(".") || text.hasPrefix("localhost") {candidate=(text.hasPrefix("localhost") || text.hasPrefix("127.0.0.1") ? "http://":"https://")+text}
            else {
                if UserDefaults.standard.string(forKey:"SearchProvider") == nil {let a=NSAlert();a.messageText="Use DuckDuckGo for searches?";a.informativeText="Search text will be sent only after you press Enter. No background suggestions are requested.";a.addButton(withTitle:"Use DuckDuckGo");a.addButton(withTitle:"Cancel");guard a.runModal() == .alertFirstButtonReturn else{return};UserDefaults.standard.set("duckduckgo",forKey:"SearchProvider")}
                var c=URLComponents(string:"https://duckduckgo.com/")!;c.queryItems=[URLQueryItem(name:"q",value:text)];candidate=c.url!.absoluteString
            }
        }
        send("navigate",["url":candidate]);SPBFocus(state.focused.activeTabId)
    }
    @objc func navigateAddress(){navigate(address.stringValue)}
    @objc func focusAddress(){window.makeFirstResponder(address);address.selectText(nil)}
    @objc func back(){SPBBack(state.focused.activeTabId)}
    @objc func forward(){SPBForward(state.focused.activeTabId)}
    @objc func reload(){if loading[state.focused.activeTabId]==true{SPBStop(state.focused.activeTabId)}else{SPBReload(state.focused.activeTabId)}}
    @objc func newTab(){send("new_tab");focusAddress()}
    @objc func selectTab(){if let id=tabs.selectedItem?.representedObject as? String{send("activate_tab",["tab_id":id],focus:true)}}
    @objc func selectContainer(){guard let id=containers.selectedItem?.representedObject as? String else{return};if id.hasPrefix("new_"){newContainer(persistent:id=="new_persistent")}else if id != state.focused.containerId{switchContainer(id)}}
    func newContainer(persistent:Bool){let a=NSAlert();a.messageText=persistent ? "New persistent container":"New temporary container";a.informativeText=persistent ? "A separate local Chromium profile. Your label can change; storage identity stays fixed.":"Shares state only with panes assigned this temporary container. Its browsing metadata is not restored after quitting.";let f=NSTextField(frame:NSRect(x:0,y:0,width:260,height:24));f.placeholderString="Container name";a.accessoryView=f;a.addButton(withTitle:"Create");a.addButton(withTitle:"Cancel");guard a.runModal() == .alertFirstButtonReturn else{render();return};let before=Set(state.containers.map(\.id));send("create_container",["name":f.stringValue.isEmpty ? (persistent ? "Personal":"Temporary"):f.stringValue,"persistence":persistent ? "persistent":"temporary"]);if let c=state.containers.first(where:{!before.contains($0.id)}){switchContainer(c.id)}}
    func switchContainer(_ id:String) {
        guard state.focused.tabs.allSatisfy({$0.url=="about:blank" && $0.activeDownloads==0}) else {showError("For safety, choose a container in a new blank pane. Switching live pages is gated until transactional before-unload validation passes.");render();return}
        send("begin_container_switch",["destination_container":id])
        do {let json=try JSONSerialization.jsonObject(with:Data(store.snapshotJson().utf8)) as! [String:Any];guard let plan=(json["pending_switches"] as? [[String:Any]])?.first,let token=plan["id"] as? String else{return};let replacements=plan["replacements"] as? [[String:Any]] ?? []
            let ready = !replacements.isEmpty && replacements.allSatisfy { replacement in guard let id=replacement["replacement_tab_id"] as? String else{return false};return surfaces[id] != nil && SPBHasTab(id) }
            if ready{send("commit_container_switch",["switch_id":token,"confirmed":true,"before_unload_resolved":true,"replacements_ready":true])}
            else{send("cancel_container_switch",["switch_id":token]);showError("Replacement Chromium instances were not ready; original container retained.")}}catch{showError(error.localizedDescription)}
    }
    @objc func closeTab(){let tab=state.focused.active;guard confirmDownloads([tab]) else{return};pendingClose.insert(tab.id);SPBCloseTab(tab.id)}
    @objc func closePane(){let pane=state.focused;guard confirmDownloads(pane.tabs) else{return};pendingPaneClose=pane.id;pendingClose.formUnion(pane.tabs.map(\.id));for tab in pane.tabs{SPBCloseTab(tab.id)}}
    func confirmDownloads(_ tabs:[Tab])->Bool {guard tabs.contains(where:{$0.activeDownloads>0}) else{return true};let a=NSAlert();a.messageText="Close with active downloads?";a.informativeText="Closing these tabs can interrupt transfers.";a.addButton(withTitle:"Keep open");a.addButton(withTitle:"Close");return a.runModal() == .alertSecondButtonReturn}
    func windowShouldClose(_ sender:NSWindow)->Bool {quitRequested();return false}
    @objc func quitRequested(){
        guard !isQuitting,confirmDownloads(state.panes.flatMap(\.tabs)) else{return}
        isQuitting=true;flushWork?.cancel();status.stringValue="Saving workspace…"
        persistenceQueue.async{[weak self] in guard let self=self else{return};do{try self.store.flush();DispatchQueue.main.async{SPBBeginQuit()}}catch{DispatchQueue.main.async{self.isQuitting=false;self.showError("Could not save before quitting: \(error.localizedDescription)")}}}
    }
    @objc func splitLeftRight(){send("split",["axis":"left_right"],focus:true)}
    @objc func splitTopBottom(){send("split",["axis":"top_bottom"],focus:true)}
    @objc func zoomPane(){send("toggle_zoom",focus:true)}
    @objc func toggleFocusMode(){focusMode.toggle();layout();status.stringValue=focusMode ? "Focus mode · ⌘⇧F to exit · \(state.container(state.focused.containerId).name)":""}
    @objc func devTools(){SPBDevTools(state.focused.activeTabId)}
    @objc func find(){let a=NSAlert();a.messageText="Find in focused tab";let f=NSTextField(frame:NSRect(x:0,y:0,width:300,height:24));a.accessoryView=f;a.addButton(withTitle:"Find");a.addButton(withTitle:"Cancel");if a.runModal() == .alertFirstButtonReturn{SPBFind(state.focused.activeTabId,f.stringValue,false)}}
    @objc func zoomIn(){SPBZoom(state.focused.activeTabId,0.5)}
    @objc func zoomOut(){SPBZoom(state.focused.activeTabId,-0.5)}
    @objc func resetZoom(){SPBZoom(state.focused.activeTabId,0)}
    @objc func appearanceChanged(){SPBSetDarkMode(window.effectiveAppearance.bestMatch(from:[.darkAqua,.aqua]) == .darkAqua);render()}
    @objc func showHelp(){let a=NSAlert();a.messageText="Pane commands";a.informativeText="Control B, then one key:\n% split left/right · \" split top/bottom\nArrows focus · Option-arrows resize\no next · ; previous · z zoom\n{ / } swap · x close pane\nEscape cancels prefix · Control B passes it to page\n\n⌘L address · ⌘T tab · ⌘W close tab · ⌘⇧F focus mode\nAgent controls are disabled pending native security gates.";a.runModal()}
    func key(_ event:NSEvent)->NSEvent? {
        guard event.window===window else{return event};if isQuitting{return nil};if bypassPrefix{bypassPrefix=false;return event}
        let controlB=event.modifierFlags.intersection(.deviceIndependentFlagsMask).contains(.control) && event.charactersIgnoringModifiers=="b"
        if !prefix {if controlB{prefix=true;status.stringValue="Pane command: %  \"  arrows  o  ;  z  {  }  x  ?";return nil};return event}
        prefix=false;status.stringValue=""
        if controlB{return event}
        if event.keyCode==53{return nil}
        let directions:[UInt16:String]=[123:"left",124:"right",125:"down",126:"up"]
        if let d=directions[event.keyCode]{if event.modifierFlags.contains(.option){send("resize_direction",["direction":d,"pixels":16])}else{send("focus_direction",["direction":d],focus:true)};return nil}
        switch event.characters {
        case "%":splitLeftRight();case "\"":splitTopBottom();case "o":send("focus_next",focus:true);case ";":send("focus_previous",focus:true);case "z":zoomPane();case "{":send("swap_adjacent",["backwards":true]);case "}":send("swap_adjacent",["backwards":false]);case "x":closePane();case "?":showHelp();default:showError("Unknown pane command; use Control B then ? for help")
        };return nil
    }
    func menus(){
        let main=NSMenu();let app=NSMenuItem();main.addItem(app);app.submenu=NSMenu();app.submenu?.addItem(withTitle:"About browsermux",action:#selector(about),keyEquivalent:"").target=self;app.submenu?.addItem(.separator());app.submenu?.addItem(withTitle:"Quit browsermux",action:#selector(quitRequested),keyEquivalent:"q").target=self
        func menu(_ title:String,_ items:[(String,Selector,String,NSEvent.ModifierFlags)]){let root=NSMenuItem();root.title=title;root.submenu=NSMenu(title:title);main.addItem(root);for (name,action,key,flags) in items{let item=NSMenuItem(title:name,action:action,keyEquivalent:key);item.keyEquivalentModifierMask=flags;item.target=self;root.submenu?.addItem(item)}}
        menu("File",[("New Tab",#selector(newTab),"t",.command),("Close Tab",#selector(closeTab),"w",.command),("Focus Address",#selector(focusAddress),"l",.command)])
        let edit=NSMenuItem();edit.title="Edit";edit.submenu=NSMenu(title:"Edit");main.addItem(edit)
        for (name,action,key) in [("Undo",Selector(("undo:")),"z"),("Cut",#selector(NSText.cut(_:)),"x"),("Copy",#selector(NSText.copy(_:)),"c"),("Paste",#selector(NSText.paste(_:)),"v"),("Select All",#selector(NSText.selectAll(_:)),"a")]{edit.submenu?.addItem(withTitle:name,action:action,keyEquivalent:key)}
        menu("View",[("Reload",#selector(reload),"r",.command),("Find",#selector(find),"f",.command),("Focus Mode",#selector(toggleFocusMode),"f",[.command,.shift]),("Developer Tools",#selector(devTools),"i",[.command,.option]),("Zoom In",#selector(zoomIn),"+",.command),("Zoom Out",#selector(zoomOut),"-",.command),("Actual Size",#selector(resetZoom),"0",.command)])
        menu("Pane",[("Split Left and Right",#selector(splitLeftRight),"d",.command),("Split Top and Bottom",#selector(splitTopBottom),"d",[.command,.shift]),("Zoom Pane",#selector(zoomPane),"z",[.command,.shift]),("Close Pane",#selector(closePane),"w",[.command,.shift]),("Pane Commands",#selector(showHelp),"",[])])
        NSApp.mainMenu=main
    }
    @objc func about(){let a=NSAlert();a.messageText="browsermux 0.1.0";a.informativeText="Swift + AppKit · Rust core · CEF \(SPBEngineVersion())\nDevelopment build. Agent access, live profile replacement and screen capture are gated. Chromium sandbox is required. This build is not notarized. See repository acceptance matrix before using sensitive accounts.";a.runModal()}
    func runSmokeTest(){
        let originalPane=state.workspace.focusedPane
        send("create_container",["name":"Fixture A","persistence":"persistent"]);send("create_container",["name":"Fixture B","persistence":"persistent"])
        guard let a=state.containers.first(where:{$0.name=="Fixture A"}),let b=state.containers.first(where:{$0.name=="Fixture B"}) else{exit(21)}
        switchContainer(a.id);splitLeftRight();switchContainer(b.id);splitTopBottom();switchContainer(a.id)
        // Split the full-height left pane, rather than quartering the right one.
        // Hosted Mac displays may constrain the window below our requested size.
        send("focus_pane",["pane_id":originalPane]);splitTopBottom()
        send("create_container",["name":"Fixture Temp","persistence":"temporary"])
        if let temp=state.containers.first(where:{$0.name=="Fixture Temp"}){switchContainer(temp.id)}
        if let url=ProcessInfo.processInfo.environment["SPB_FIXTURE_URL"] {
            var wroteA=false
            for (index,pane) in state.panes.enumerated() {
                let value=pane.containerId==a.id ? "A":(pane.containerId==b.id ? "B":"T")
                let read=value=="A" && wroteA
                if value=="A"{wroteA=true}
                let target="\(url)?expected=\(value)&pane=\(index)&mode=\(read ? "read":"write")"
                if read {DispatchQueue.main.asyncAfter(deadline:.now()+3){[weak self] in self?.send("navigate",["tab_id":pane.activeTabId,"url":target])}}
                else {send("navigate",["tab_id":pane.activeTabId,"url":target])}
            }
        }
        DispatchQueue.main.asyncAfter(deadline:.now()+8){[weak self] in guard let self=self else{return};let success=self.state.panes.count==4 && SPBLiveBrowserCount()==4;print("NATIVE_SMOKE panes=\(self.state.panes.count) browsers=\(SPBLiveBrowserCount()) sandbox_required=true");if !success{exit(22)};self.quitRequested()}
    }
}
