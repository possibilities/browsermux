#include "include/cef_app.h"
#include "include/cef_sandbox_mac.h"
#include "include/wrapper/cef_library_loader.h"
#ifndef CEF_USE_SANDBOX
#error "A sandbox-enabled helper is required"
#endif
int main(int argc, char* argv[]) {
  CefScopedSandboxContext sandbox;
  if (!sandbox.Initialize(argc, argv)) return 70;
  CefScopedLibraryLoader library;
  if (!library.LoadInHelper()) return 71;
  return CefExecuteProcess(CefMainArgs(argc, argv), nullptr, nullptr);
}
