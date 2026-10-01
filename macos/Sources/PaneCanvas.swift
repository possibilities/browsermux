import AppKit
final class FlippedView:NSView {override var isFlipped:Bool {true}}
final class PaneView:NSView {
    override var isFlipped:Bool {true}
    let header=NSTextField(labelWithString:"")
    let surface=FlippedView()
    let search=NSSearchField()
    var select:(()->Void)?
    var submit:((String)->Void)?
    override init(frame:NSRect) {
        super.init(frame:frame);wantsLayer=true
        header.font = .systemFont(ofSize:11);header.textColor = .secondaryLabelColor
        header.lineBreakMode = .byTruncatingTail
        search.placeholderString="Enter URL or search…";search.font = .systemFont(ofSize:22)
        search.target=self;search.action=#selector(searchEntered)
        addSubview(header);addSubview(surface);addSubview(search)
        setAccessibilityRole(.group)
    }
    required init?(coder:NSCoder){fatalError("init(coder:) has not been implemented")}
    override func mouseDown(with event:NSEvent){select?();super.mouseDown(with:event)}
    @objc func searchEntered(){select?();submit?(search.stringValue)}
    func update(pane:Pane,container:Container,split:Bool,focused:Bool) {
        header.isHidden = !split
        header.stringValue="\(container.name) · \(pane.active.title.isEmpty ? "New Tab" : pane.active.title)"
        setAccessibilityLabel("\(focused ? "Focused pane" : "Pane"), container \(container.name), \(pane.active.title)")
        layer?.borderWidth=split && focused ? 1 : 0
        layer?.borderColor=NSColor.controlAccentColor.withAlphaComponent(0.6).cgColor
        layer?.backgroundColor=NSColor.textBackgroundColor.cgColor
        search.isHidden=pane.active.url != "about:blank"
        needsLayout=true
    }
    override func layout(){super.layout();let h:CGFloat=header.isHidden ? 0:22;header.frame=NSRect(x:10,y:1,width:bounds.width-20,height:20);surface.frame=NSRect(x:0,y:h,width:bounds.width,height:max(0,bounds.height-h));search.frame=NSRect(x:bounds.width*0.17,y:bounds.height*0.5-24,width:bounds.width*0.66,height:48)}
}
final class DividerView:NSView {
    override var isFlipped:Bool{true}
    var splitId="";var axis="";var region=NSRect.zero;var ratio:CGFloat=0.5
    var change:((String,CGFloat)->Void)?
    override var acceptsFirstResponder:Bool{true}
    override init(frame:NSRect){super.init(frame:frame);setAccessibilityRole(.splitter);setAccessibilityLabel("Pane divider")}
    required init?(coder:NSCoder){fatalError("init(coder:) has not been implemented")}
    override func draw(_ dirtyRect:NSRect){NSColor.separatorColor.setFill();bounds.insetBy(dx:axis=="left_right" ? 2:0,dy:axis=="left_right" ? 0:2).fill()}
    override func resetCursorRects(){addCursorRect(bounds,cursor:axis=="left_right" ? .resizeLeftRight:.resizeUpDown)}
    override func mouseDown(with event:NSEvent){window?.makeFirstResponder(self);drag(event)}
    override func mouseDragged(with event:NSEvent){drag(event)}
    private func drag(_ event:NSEvent){guard let p=superview?.convert(event.locationInWindow,from:nil) else{return};let r=axis=="left_right" ? (p.x-region.minX)/(region.width-6):(p.y-region.minY)/(region.height-6);change?(splitId,min(0.99,max(0.01,r)))}
    override func accessibilityValue()->Any? {Double(ratio*100)}
    override func accessibilityPerformIncrement()->Bool {change?(splitId,min(0.99,ratio+0.025));return true}
    override func accessibilityPerformDecrement()->Bool {change?(splitId,max(0.01,ratio-0.025));return true}
    override func keyDown(with event:NSEvent){switch event.keyCode {case 123,126:_=accessibilityPerformDecrement();case 124,125:_=accessibilityPerformIncrement();default:super.keyDown(with:event)}}
}
final class PaneCanvas:NSView {
    override var isFlipped:Bool{true}
    var panes:[String:PaneView]=[:];var dividers:[String:DividerView]=[:]
    var select:((String)->Void)?;var submit:((String)->Void)?;var resize:((String,CGFloat)->Void)?
    func update(_ state:Snapshot) {
        frame.size=NSSize(width:state.contentSize.width,height:state.contentSize.height)
        for pane in state.panes {
            let view=panes[pane.id] ?? PaneView(frame:.zero)
            if panes[pane.id]==nil {panes[pane.id]=view;addSubview(view);view.select={[weak self] in self?.select?(pane.id)};view.submit={[weak self] in self?.submit?($0)}}
            if let layout=state.layout.first(where:{$0.paneId==pane.id}) {view.frame=layout.rect.rect;view.isHidden = !layout.visible}
            view.update(pane:pane,container:state.container(pane.containerId),split:state.panes.count>1,focused:pane.id==state.workspace.focusedPane)
        }
        for id in Array(panes.keys) where !state.panes.contains(where:{$0.id==id}) {panes.removeValue(forKey:id)?.removeFromSuperview()}
        var keep=Set<String>()
        func layout(_ node:Tree) -> NSRect {
            switch node {
            case .leaf(let id):return state.layout.first(where:{$0.paneId==id})?.rect.rect ?? .zero
            case .split(let id,let axis,let ratio,let first,let second):
                let a=layout(first),b=layout(second),whole=a.union(b)
                keep.insert(id);let view=dividers[id] ?? DividerView(frame:.zero)
                if dividers[id]==nil {dividers[id]=view;addSubview(view)}
                view.splitId=id;view.axis=axis;view.region=whole;view.ratio=ratio;view.change=resize
                view.isHidden=state.workspace.zoomedPane != nil
                view.frame=axis=="left_right" ? NSRect(x:a.maxX,y:whole.minY,width:6,height:whole.height):NSRect(x:whole.minX,y:a.maxY,width:whole.width,height:6)
                view.needsDisplay=true;return whole
            }
        }
        _=layout(state.workspace.root)
        for id in Array(dividers.keys) where !keep.contains(id) {dividers.removeValue(forKey:id)?.removeFromSuperview()}
    }
}
