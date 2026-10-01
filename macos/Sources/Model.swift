import AppKit
struct Container: Decodable { let id: String; let name: String; let persistence: String; let storageLocator: String? }
struct Tab: Decodable { let id: String; let url: String; let title: String; let navigationGeneration: UInt64; let lifecycle: String; let reopenAllowed: Bool; let activeDownloads: UInt32 }
struct Pane: Decodable { let id: String; let containerId: String; let generation: UInt64; let tabs: [Tab]; let activeTabId: String; var active: Tab { tabs.first(where: {$0.id == activeTabId})! } }
struct Dimensions: Decodable { let width: CGFloat; let height: CGFloat }
struct Bounds: Decodable { let x: CGFloat; let y: CGFloat; let width: CGFloat; let height: CGFloat; var rect: NSRect {NSRect(x:x,y:y,width:width,height:height)} }
struct PaneLayout: Decodable { let paneId: String; let rect: Bounds; let visible: Bool }
indirect enum Tree: Decodable {
    case leaf(String)
    case split(id:String, axis:String, ratio:CGFloat, first:Tree, second:Tree)
    private enum CodingKeys:String,CodingKey {case kind,paneId,id,axis,ratio,first,second}
    init(from decoder:Decoder)throws {
        let c=try decoder.container(keyedBy:CodingKeys.self)
        if try c.decode(String.self,forKey:.kind)=="leaf" {self = .leaf(try c.decode(String.self,forKey:.paneId))}
        else {self = .split(id:try c.decode(String.self,forKey:.id),axis:try c.decode(String.self,forKey:.axis),ratio:try c.decode(CGFloat.self,forKey:.ratio),first:try c.decode(Tree.self,forKey:.first),second:try c.decode(Tree.self,forKey:.second))}
    }
    var leaves:[String] {switch self {case .leaf(let id):return [id];case .split(_,_,_,let a,let b):return a.leaves+b.leaves}}
}
struct Workspace: Decodable { let id:String; let root:Tree; let focusedPane:String; let lastFocusedPane:String?; let zoomedPane:String? }
struct Snapshot: Decodable {
    let revision:UInt64;let workspace:Workspace;let containers:[Container];let panes:[Pane];let contentSize:Dimensions;let layout:[PaneLayout]
    var focused:Pane {panes.first(where:{$0.id==workspace.focusedPane})!}
    func container(_ id:String)->Container {containers.first(where:{$0.id==id})!}
    func tab(_ id:String)->Tab? {panes.flatMap(\.tabs).first(where:{$0.id==id})}
}
struct Effect: Decodable {let type:String;let tabId:String?;let containerId:String?;let url:String?;let paneId:String?}
struct ResultEnvelope: Decodable {let snapshot:Snapshot;let effects:[Effect]}
let decoder:JSONDecoder = {let d=JSONDecoder();d.keyDecodingStrategy = .convertFromSnakeCase;return d}()
func decode<T:Decodable>(_ type:T.Type,_ json:String)throws->T {try decoder.decode(type,from:Data(json.utf8))}
func encodeCommand(_ type:String,_ fields:[String:Any]=[:])throws->String {var f=fields;f["type"]=type;return String(data:try JSONSerialization.data(withJSONObject:f,options:[.sortedKeys]),encoding:.utf8)!}
