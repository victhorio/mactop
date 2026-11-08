import AppKit

@main
@MainActor
final class MactopApplication: NSObject, NSApplicationDelegate {
  private var store: MonitorStore?
  private var status: StatusController?
  private var terminating = false
  #if DEBUG
    private var preview: NativePreview?
  #endif

  static func main() {
    let application = NSApplication.shared
    let delegate = MactopApplication()
    application.setActivationPolicy(.accessory)
    application.delegate = delegate
    withExtendedLifetime(delegate) { application.run() }
  }

  static func quit() {
    // Terminate outside a Swift main-executor job so AppKit's termination loop
    // can run the asynchronous sampler shutdown on that executor.
    NSApp.perform(#selector(NSApplication.terminate(_:)), with: nil, afterDelay: 0)
  }

  func applicationDidFinishLaunching(_ notification: Notification) {
    let arguments = ProcessInfo.processInfo.arguments
    let store = MonitorStore(demo: arguments.contains("--demo"))
    let status = StatusController(store: store)
    self.store = store
    self.status = status
    #if DEBUG
      if arguments.contains("--capture") {
        preview = NativePreview(store: store, status: status, arguments: arguments)
        return
      }
    #endif
    store.start()
  }

  func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
    guard !terminating else { return .terminateLater }
    terminating = true
    Task {
      await store?.stop()
      sender.reply(toApplicationShouldTerminate: true)
    }
    return .terminateLater
  }
}
