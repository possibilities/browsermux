#import "BrowserEngine.h"
#include <crt_externs.h>
#include <map>
#include <memory>
#include <set>
#include <string>
#include "include/cef_app.h"
#include "include/cef_application_mac.h"
#include "include/cef_browser.h"
#include "include/cef_client.h"
#include "include/cef_command_line.h"
#include "include/cef_parser.h"
#include "include/cef_version.h"
#include "include/wrapper/cef_helpers.h"
#include "include/wrapper/cef_library_loader.h"

namespace {
class Client;
std::unique_ptr<CefScopedLibraryLoader> loader;
std::map<std::string, CefRefPtr<CefRequestContext>> contexts;
std::map<std::string, std::string> contextPaths;
std::set<std::string> retiring;
std::map<std::string, CefRefPtr<Client>> clients;
void (^events)(NSDictionary*) = nil;
bool initialized=false, quitting=false;
std::string focusedTab;
std::map<int,CefRefPtr<CefBrowser>> toolsBrowsers;
NSString* rootPath=nil;
NSString* ns(const CefString& s) { return [NSString stringWithUTF8String:s.ToString().c_str()] ?: @""; }
void emit(NSString* type, const std::string& tab, NSDictionary* payload=@{}) {
  if (!events) return;
  NSMutableDictionary* e=[payload mutableCopy]; e[@"type"]=type; e[@"tab_id"]=[NSString stringWithUTF8String:tab.c_str()];
  events(e);
}
bool safe(const CefString& input) {
  if (input == "about:blank") return true;
  CefURLParts p; if (!CefParseURL(input,p)) return false;
  auto scheme=CefString(&p.scheme).ToString();
  return (scheme=="http" || scheme=="https") && p.username.length==0 && p.password.length==0;
}
class ToolsClient final : public CefClient, public CefLifeSpanHandler {
 public:
  explicit ToolsClient(std::string identity):identity_(std::move(identity)){}
  CefRefPtr<CefLifeSpanHandler> GetLifeSpanHandler() override {return this;}
  void OnAfterCreated(CefRefPtr<CefBrowser> b) override {
    toolsBrowsers[b->GetIdentifier()]=b;
    NSView* view=(__bridge NSView*)b->GetHost()->GetWindowHandle();
    view.window.title=[NSString stringWithFormat:@"DevTools · %s",identity_.c_str()];
  }
  void OnBeforeClose(CefRefPtr<CefBrowser> b) override {toolsBrowsers.erase(b->GetIdentifier());if(quitting && clients.empty() && toolsBrowsers.empty())CefQuitMessageLoop();}
 private:
  std::string identity_;
  IMPLEMENT_REFCOUNTING(ToolsClient);
};
class Client final : public CefClient, public CefLifeSpanHandler, public CefDisplayHandler,
  public CefLoadHandler, public CefRequestHandler, public CefPermissionHandler,
  public CefDownloadHandler, public CefJSDialogHandler, public CefFocusHandler {
 public:
  const std::string tab, container;
  CefRefPtr<CefBrowser> browser;
  bool closeRequested=false;
  bool reopen=false;
  uint64_t epoch=0;
  bool requestedNavigation=false, loading=false;
  void event(NSString* type, NSDictionary* payload=@{}) {NSMutableDictionary* p=[payload mutableCopy];p[@"navigation_epoch"]=@(epoch);emit(type,tab,p);}
  explicit Client(std::string t,std::string c):tab(std::move(t)),container(std::move(c)){}
  CefRefPtr<CefLifeSpanHandler> GetLifeSpanHandler() override {return this;}
  CefRefPtr<CefDisplayHandler> GetDisplayHandler() override {return this;}
  CefRefPtr<CefLoadHandler> GetLoadHandler() override {return this;}
  CefRefPtr<CefRequestHandler> GetRequestHandler() override {return this;}
  CefRefPtr<CefPermissionHandler> GetPermissionHandler() override {return this;}
  CefRefPtr<CefDownloadHandler> GetDownloadHandler() override {return this;}
  CefRefPtr<CefJSDialogHandler> GetJSDialogHandler() override {return this;}
  CefRefPtr<CefFocusHandler> GetFocusHandler() override {return this;}
  bool OnSetFocus(CefRefPtr<CefBrowser>,FocusSource source) override {return source==FOCUS_SOURCE_NAVIGATION && focusedTab!=tab;}
  void OnGotFocus(CefRefPtr<CefBrowser>) override {focusedTab=tab;emit(@"focused",tab);}
  void OnAfterCreated(CefRefPtr<CefBrowser> b) override {CEF_REQUIRE_UI_THREAD();browser=b;emit(@"created",tab);}
  bool DoClose(CefRefPtr<CefBrowser> b) override {
    // A leaf browser must never send performClose: to the entire workspace window.
    NSView* view=(__bridge NSView*)b->GetHost()->GetWindowHandle();
    dispatch_async(dispatch_get_main_queue(), ^{[view removeFromSuperview];});
    return true;
  }
  void OnBeforeClose(CefRefPtr<CefBrowser>) override {
    CEF_REQUIRE_UI_THREAD(); auto keep=CefRefPtr<Client>(this); browser=nullptr;
    const auto id=tab; clients.erase(id); emit(@"closed",id);
    if(retiring.count(container)) {bool live=false;for(const auto&[t,c]:clients)if(c->container==container)live=true;if(!live){contexts.erase(container);contextPaths.erase(container);retiring.erase(container);}}
    if(quitting && clients.empty() && toolsBrowsers.empty()) CefQuitMessageLoop();
  }
  bool OnBeforePopup(CefRefPtr<CefBrowser>,CefRefPtr<CefFrame>,int,const CefString& url,const CefString&,WindowOpenDisposition,bool gesture,const CefPopupFeatures&,CefWindowInfo&,CefRefPtr<CefClient>&,CefBrowserSettings&,CefRefPtr<CefDictionaryValue>&,bool*) override {
    // Host opens a pane-owned tab in the SAME canonical context. No global fallback.
    if(gesture && safe(url)) emit(@"popup",tab,@{@"url":ns(url),@"container_id":[NSString stringWithUTF8String:container.c_str()]});
    else emit(@"popup_blocked",tab);
    return true;
  }
  bool OnBeforeBrowse(CefRefPtr<CefBrowser>,CefRefPtr<CefFrame> frame,CefRefPtr<CefRequest> request,bool,bool) override {
    if(!safe(request->GetURL())) {emit(@"blocked_navigation",tab);return true;}
    if(frame->IsMain()) {if(!requestedNavigation)++epoch;requestedNavigation=false;reopen=request->GetMethod()=="GET";event(@"navigation_started");}
    return false;
  }
  bool OnCertificateError(CefRefPtr<CefBrowser>,cef_errorcode_t,const CefString&,CefRefPtr<CefSSLInfo>,CefRefPtr<CefCallback>) override {return false;}
  void OnTitleChange(CefRefPtr<CefBrowser>,const CefString& title) override {if(!loading)event(@"title",@{@"title":ns(title)});}
  void OnAddressChange(CefRefPtr<CefBrowser>,CefRefPtr<CefFrame> frame,const CefString& url) override {
    if(frame->IsMain()) event(@"address",@{@"url":ns(url),@"reopen_allowed":@(reopen)});
  }
  void OnLoadingStateChange(CefRefPtr<CefBrowser>,bool value,bool back,bool forward) override {loading=value;event(@"loading",@{@"loading":@(value),@"can_back":@(back),@"can_forward":@(forward)});}
  void OnLoadError(CefRefPtr<CefBrowser>,CefRefPtr<CefFrame> frame,ErrorCode code,const CefString&,const CefString&) override {if(frame->IsMain() && code!=ERR_ABORTED)emit(@"load_error",tab,@{@"code":@(code)});}
  void OnRenderProcessTerminated(CefRefPtr<CefBrowser>,TerminationStatus,int,const CefString&) override {emit(@"renderer_failed",tab);}
  bool OnRequestMediaAccessPermission(CefRefPtr<CefBrowser>,CefRefPtr<CefFrame> frame,const CefString& origin,uint32_t requested,CefRefPtr<CefMediaAccessCallback> callback) override {
    // Screen capture remains disabled until OS source-picker and stop/revoke tests pass.
    if(requested & (CEF_MEDIA_PERMISSION_DESKTOP_AUDIO_CAPTURE|CEF_MEDIA_PERMISSION_DESKTOP_VIDEO_CAPTURE)) {callback->Cancel();emit(@"permission_denied",tab,@{@"reason":@"Screen capture is not enabled in this development build"});return true;}
    constexpr uint32_t supported=CEF_MEDIA_PERMISSION_DEVICE_AUDIO_CAPTURE|CEF_MEDIA_PERMISSION_DEVICE_VIDEO_CAPTURE;
    if((requested & ~supported)!=0) {callback->Cancel();return true;}
    NSAlert* alert=[[NSAlert alloc] init]; alert.messageText=@"Allow camera or microphone?";
    CefURLParts topParts;CefParseURL(browser->GetMainFrame()->GetURL(),topParts);
    NSString* topOrigin=[NSString stringWithFormat:@"%@://%@%@%@",ns(CefString(&topParts.scheme)),ns(CefString(&topParts.host)),topParts.port.length ? @":":@"",ns(CefString(&topParts.port))];
    alert.informativeText=[NSString stringWithFormat:@"Requesting origin: %@\nTop-level origin: %@\nContainer: %s\nRequested: %@%@",ns(origin),topOrigin,container.c_str(),(requested&CEF_MEDIA_PERMISSION_DEVICE_AUDIO_CAPTURE)?@"microphone ":@"",(requested&CEF_MEDIA_PERMISSION_DEVICE_VIDEO_CAPTURE)?@"camera":@""];
    [alert addButtonWithTitle:@"Deny"];[alert addButtonWithTitle:@"Allow once"];
    NSView* view=(__bridge NSView*)browser->GetHost()->GetWindowHandle();
    [alert beginSheetModalForWindow:view.window completionHandler:^(NSModalResponse response){if(response==NSAlertSecondButtonReturn)callback->Continue(requested);else callback->Cancel();}];
    return true;
  }
  bool OnShowPermissionPrompt(CefRefPtr<CefBrowser>,uint64_t,const CefString& origin,uint32_t,CefRefPtr<CefPermissionPromptCallback> callback) override {
    callback->Continue(CEF_PERMISSION_RESULT_DENY);emit(@"permission_denied",tab,@{@"origin":ns(origin)});return true;
  }
  bool OnBeforeDownload(CefRefPtr<CefBrowser>,CefRefPtr<CefDownloadItem>,const CefString& suggested,CefRefPtr<CefBeforeDownloadCallback> callback) override {
    NSSavePanel* panel=[NSSavePanel savePanel];panel.nameFieldStringValue=[ns(suggested) lastPathComponent];
    panel.title=[NSString stringWithFormat:@"Save download · %s",container.c_str()];
    [panel beginWithCompletionHandler:^(NSModalResponse response){if(response==NSModalResponseOK && panel.URL)callback->Continue(CefString(panel.URL.path.UTF8String),false);}];return true;
  }
  void OnDownloadUpdated(CefRefPtr<CefBrowser>,CefRefPtr<CefDownloadItem> item,CefRefPtr<CefDownloadItemCallback>) override {
    emit(@"download",tab,@{@"id":@(item->GetId()),@"active":@(item->IsInProgress()),@"complete":@(item->IsComplete()),@"percent":@(item->GetPercentComplete()),@"name":ns(item->GetSuggestedFileName())});
  }
  bool OnBeforeUnloadDialog(CefRefPtr<CefBrowser> b,const CefString&,bool,CefRefPtr<CefJSDialogCallback> callback) override {
    NSAlert* alert=[[NSAlert alloc]init];alert.messageText=@"Leave this page?";alert.informativeText=@"The page may have unsaved changes.";[alert addButtonWithTitle:@"Stay"];[alert addButtonWithTitle:@"Leave"];
    NSView* view=(__bridge NSView*)b->GetHost()->GetWindowHandle();
    auto self=CefRefPtr<Client>(this);
    [alert beginSheetModalForWindow:view.window completionHandler:^(NSModalResponse result){bool leave=result==NSAlertSecondButtonReturn;callback->Continue(leave,CefString());if(!leave){if(self->closeRequested){self->closeRequested=false;quitting=false;emit(@"close_cancelled",self->tab);}else if(self->browser){self->event(@"navigation_cancelled",@{@"url":ns(self->browser->GetMainFrame()->GetURL())});}}}];return true;
  }
  IMPLEMENT_REFCOUNTING(Client);
};
CefRefPtr<Client> get(NSString* id) {auto it=clients.find(id.UTF8String);return it==clients.end()?nullptr:it->second;}
}
@interface SPBApplication () <CefAppProtocol>
@property(nonatomic) BOOL handlingSendEvent;
@end
@implementation SPBApplication
- (BOOL)isHandlingSendEvent {return _handlingSendEvent;}
- (void)sendEvent:(NSEvent*)event {CefScopedSendingEvent scope;[super sendEvent:event];}
- (void)terminate:(id)sender {[[NSNotificationCenter defaultCenter]postNotificationName:@"SPBQuitRequested" object:nil];}
@end
BOOL SPBInitialize(NSString* dataRoot) {
  NSCAssert([NSThread isMainThread],@"CEF must initialize on main thread");
  if(initialized)return NO;
  [SPBApplication sharedApplication];
  if(![NSApp isKindOfClass:[SPBApplication class]])return NO;
  rootPath=[dataRoot stringByAppendingPathComponent:@"engine"];
  NSError* error=nil;[[NSFileManager defaultManager]createDirectoryAtPath:rootPath withIntermediateDirectories:YES attributes:@{NSFilePosixPermissions:@0700} error:&error];if(error)return NO;
  loader=std::make_unique<CefScopedLibraryLoader>();if(!loader->LoadInMain())return NO;
  CefSettings settings;settings.no_sandbox=false;settings.command_line_args_disabled=true;
  CefString(&settings.root_cache_path)=rootPath.UTF8String;settings.persist_session_cookies=false;
  settings.log_severity=LOGSEVERITY_DISABLE;
  initialized=CefInitialize(CefMainArgs(*_NSGetArgc(),*_NSGetArgv()),settings,nullptr,nullptr);
  return initialized;
}
void SPBSetEventHandler(void(^handler)(NSDictionary*)) {events=[handler copy];}
void SPBRunLoop(void) {if(initialized)CefRunMessageLoop();}
void SPBShutdown(void) {NSCAssert(clients.empty() && toolsBrowsers.empty(),@"Live browsers prevent shutdown");contexts.clear();contextPaths.clear();events=nil;if(initialized)CefShutdown();initialized=false;loader.reset();}
BOOL SPBCreateTab(NSString* tabID,NSString* containerID,NSString* cachePath,NSView* host) {
  CEF_REQUIRE_UI_THREAD();if(quitting || !initialized || get(tabID))return NO;
  const std::string key=containerID.UTF8String;
  const std::string requested=cachePath.UTF8String;
  if(!contexts.count(key)) {
    CefRequestContextSettings s;
    if(!requested.empty()) {
      NSString* expected=[rootPath stringByAppendingPathComponent:[@"profiles" stringByAppendingPathComponent:containerID]];
      if(![cachePath isEqualToString:expected])return NO;
      NSError* e=nil;[[NSFileManager defaultManager]createDirectoryAtPath:cachePath withIntermediateDirectories:YES attributes:@{NSFilePosixPermissions:@0700} error:&e];if(e)return NO;
      CefString(&s.cache_path)=requested;
    }
    s.persist_session_cookies=false;
    contexts[key]=CefRequestContext::CreateContext(s,nullptr);contextPaths[key]=requested;
  } else if(contextPaths[key]!=requested)return NO;
  if(!contexts[key])return NO;
  CefWindowInfo info;info.SetAsChild((__bridge CefWindowHandle)host,CefRect(0,0,host.bounds.size.width,host.bounds.size.height));info.runtime_style=CEF_RUNTIME_STYLE_ALLOY;
  CefBrowserSettings settings;auto client=CefRefPtr<Client>(new Client(tabID.UTF8String,key));clients[tabID.UTF8String]=client;
  auto browser=CefBrowserHost::CreateBrowserSync(info,client,"about:blank",settings,nullptr,contexts[key]);
  if(!browser){clients.erase(tabID.UTF8String);return NO;}
  NSView* view=(__bridge NSView*)browser->GetHost()->GetWindowHandle();view.autoresizingMask=NSViewWidthSizable|NSViewHeightSizable;
  return YES;
}
void SPBCloseTab(NSString* id){auto c=get(id);if(c&&c->browser&&!c->closeRequested){c->closeRequested=true;c->browser->GetHost()->CloseDevTools();c->browser->GetHost()->CloseBrowser(false);}}
void SPBBeginQuit(void){quitting=true;auto toolsCopy=toolsBrowsers;for(const auto&[id,b]:toolsCopy)b->GetHost()->CloseBrowser(false);if(clients.empty() && toolsBrowsers.empty()){CefQuitMessageLoop();return;}auto copy=clients;for(const auto&[id,c]:copy)SPBCloseTab([NSString stringWithUTF8String:id.c_str()]);}
uint64_t SPBNavigate(NSString* id,NSString* url){auto c=get(id);if(c&&c->browser&&safe(CefString(url.UTF8String))){++c->epoch;c->requestedNavigation=true;c->browser->GetMainFrame()->LoadURL(url.UTF8String);return c->epoch;}return 0;}
BOOL SPBHasTab(NSString* id){auto c=get(id);return c&&c->browser&&!c->closeRequested;}
void SPBBack(NSString* id){auto c=get(id);if(c&&c->browser)c->browser->GoBack();}
void SPBForward(NSString* id){auto c=get(id);if(c&&c->browser)c->browser->GoForward();}
void SPBReload(NSString* id){auto c=get(id);if(c&&c->browser)c->browser->Reload();}
void SPBStop(NSString* id){auto c=get(id);if(c&&c->browser)c->browser->StopLoad();}
void SPBFocus(NSString* id){focusedTab=id.UTF8String;auto c=get(id);if(c&&c->browser)c->browser->GetHost()->SetFocus(true);}
void SPBDevTools(NSString* id){auto c=get(id);if(c&&c->browser){CefWindowInfo w;CefBrowserSettings s;auto dev=CefRefPtr<ToolsClient>(new ToolsClient(c->tab+" · "+c->container));c->browser->GetHost()->ShowDevTools(w,dev,s,CefPoint());}}
void SPBFind(NSString* id,NSString* text,BOOL next){auto c=get(id);if(c&&c->browser)c->browser->GetHost()->Find(text.UTF8String,true,false,next);}
void SPBZoom(NSString* id,double delta){auto c=get(id);if(c&&c->browser){auto h=c->browser->GetHost();h->SetZoomLevel(delta==0?0:h->GetZoomLevel()+delta);}}
void SPBSetDarkMode(BOOL dark){for(const auto&[id,c]:contexts)c->SetChromeColorScheme(dark?CEF_COLOR_VARIANT_DARK:CEF_COLOR_VARIANT_LIGHT,0);}
void SPBRetireContainer(NSString* id){const std::string key=id.UTF8String;retiring.insert(key);for(const auto&[t,c]:clients)if(c->container==key)return;contexts.erase(key);contextPaths.erase(key);retiring.erase(key);}
NSInteger SPBLiveBrowserCount(void){return clients.size();}
NSString* SPBEngineVersion(void){return @CEF_VERSION;}
