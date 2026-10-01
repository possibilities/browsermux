#import <Cocoa/Cocoa.h>
NS_ASSUME_NONNULL_BEGIN
/// CEF implementation is entirely Objective-C++; Swift never owns a CEF pointer.
#ifdef __cplusplus
extern "C" {
#endif
BOOL SPBInitialize(NSString *dataRoot);
void SPBRunLoop(void);
void SPBShutdown(void);
void SPBBeginQuit(void);
typedef void (^SPBEventHandler)(NSDictionary<NSString *, id> *event);
void SPBSetEventHandler(SPBEventHandler handler);
BOOL SPBCreateTab(NSString *tabID, NSString *containerID, NSString *cachePath, NSView *host);
void SPBCloseTab(NSString *tabID);
uint64_t SPBNavigate(NSString *tabID, NSString *url);
BOOL SPBHasTab(NSString *tabID);
void SPBBack(NSString *tabID);
void SPBForward(NSString *tabID);
void SPBReload(NSString *tabID);
void SPBStop(NSString *tabID);
void SPBFocus(NSString *tabID);
void SPBDevTools(NSString *tabID);
void SPBFind(NSString *tabID, NSString *text, BOOL next);
void SPBZoom(NSString *tabID, double delta);
void SPBSetDarkMode(BOOL dark);
void SPBRetireContainer(NSString *containerID);
NSInteger SPBLiveBrowserCount(void);
NSString *SPBEngineVersion(void);
#ifdef __cplusplus
}
#endif
@interface SPBApplication : NSApplication
@end
NS_ASSUME_NONNULL_END
