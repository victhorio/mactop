#if DEBUG
  import AppKit
  import ScreenCaptureKit

  @MainActor
  final class NativePreview {
    init(store: MonitorStore, status: StatusController, arguments: [String]) {
      Task {
        do {
          guard #available(macOS 14.4, *) else {
            throw CollectionPreviewError("Window capture requires macOS 14.4 or later.")
          }
          guard let path = Self.argument("--capture", in: arguments) else {
            throw CollectionPreviewError("--capture requires a PNG path.")
          }
          let panel =
            MetricPanel(rawValue: Self.argument("--panel", in: arguments) ?? "cpu") ?? .cpu
          if let appearance = Self.argument("--appearance", in: arguments) {
            NSApp.appearance = NSAppearance(named: appearance == "dark" ? .darkAqua : .aqua)
            // Status-item popovers inherit the menu bar's appearance, not the app's.
            for item in status.items { item.popover.appearance = NSApp.appearance }
          }
          try await Task.sleep(for: .milliseconds(200))
          status.show(panel)
          guard let item = status.items.first(where: { $0.panel == panel }),
            let window = item.popover.contentViewController?.view.window
          else { throw CollectionPreviewError("The popover did not open.") }
          try await Task.sleep(for: .milliseconds(250))
          print(
            "Appearance: app \(NSApp.effectiveAppearance.name.rawValue), "
              + "popover \(item.popover.effectiveAppearance.name.rawValue), "
              + "window \(window.effectiveAppearance.name.rawValue)"
          )
          if let appearance = Self.argument("--appearance", in: arguments),
            window.effectiveAppearance.bestMatch(from: [.aqua, .darkAqua])
              != (appearance == "dark" ? .darkAqua : .aqua)
          {
            throw CollectionPreviewError("The popover did not adopt the requested appearance.")
          }
          let stem = (path as NSString).deletingPathExtension
          try await Self.capture(window, to: stem + "-loading.png")
          store.start()
          let deadline = ContinuousClock.now + .seconds(10)
          while store.reading == nil && store.error == nil && .now < deadline {
            try await Task.sleep(for: .milliseconds(50))
          }
          guard store.reading != nil else {
            try await Task.sleep(for: .milliseconds(300))
            try await Self.capture(window, to: path)
            throw CollectionPreviewError(store.error ?? "No reading arrived within ten seconds.")
          }
          try await Task.sleep(for: .milliseconds(300))
          try await Self.capture(window, to: path)
          let populatedSize = window.frame.size
          print("\(panel.rawValue): \(populatedSize), \(item.statusItem.button?.toolTip ?? "")")
          for entry in status.items {
            guard let image = entry.statusItem.button?.image?.tiffRepresentation,
              let bitmap = NSBitmapImageRep(data: image),
              let data = bitmap.representation(using: .png, properties: [:])
            else { throw CollectionPreviewError("Could not capture the status-item image.") }
            try data.write(to: URL(fileURLWithPath: stem + "-\(entry.panel.rawValue)-badge.png"))
            if let button = entry.statusItem.button {
              print(
                "\(entry.panel.rawValue) badge: \(button.bounds), image \(button.cell!.imageRect(forBounds: button.bounds))"
              )
            }
          }

          if arguments.contains("--capture-menu") {
            try await Self.captureMenu(item, to: stem + "-menu.png")
            let menu = item.makeMenu()
            menu.performActionForItem(at: menu.items.firstIndex { $0.tag == 5000 }!)
            try await Task.sleep(for: .milliseconds(200))
            let slowSequence = store.reading!.sequence
            let slowDeadline = ContinuousClock.now + .seconds(6)
            while store.reading!.sequence == slowSequence && .now < slowDeadline {
              try await Task.sleep(for: .milliseconds(20))
            }
            guard store.reading!.sequence == slowSequence + 1, !store.isStale else {
              throw CollectionPreviewError(
                "Selecting 0.2 Hz did not deliver a fresh native reading.")
            }
            menu.performActionForItem(at: menu.items.firstIndex { $0.tag == 250 }!)
            guard store.frequency == .four,
              status.items.allSatisfy({ $0.makeMenu().item(withTag: 250)?.state == .on })
            else { throw CollectionPreviewError("The menu did not share its frequency selection.") }
            let sequence = store.reading!.sequence
            let deadline = ContinuousClock.now + .seconds(3)
            while store.reading!.sequence < sequence + 4 && .now < deadline {
              try await Task.sleep(for: .milliseconds(20))
            }
            guard store.reading!.sequence >= sequence + 4 else {
              throw CollectionPreviewError("Selecting 4 Hz did not speed up native collection.")
            }
            status.show(panel)
            try await Task.sleep(for: .milliseconds(300))
            try await Self.capture(
              item.popover.contentViewController!.view.window!, to: stem + "-fast.png")
            try await Self.captureMenu(item, to: stem + "-menu-fast.png")
            status.show(panel)
            try await Task.sleep(for: .milliseconds(300))
            print("Both menus share selection; native collection advanced at 4 Hz: passed")
          }

          if arguments.contains("--check-transitions") {
            await store.stop()
            store.fail("Preview: collection interrupted. The last good reading is retained.")
            try await Task.sleep(for: .milliseconds(300))
            try await Self.capture(window, to: stem + "-error.png")
            guard window.frame.height > populatedSize.height else {
              throw CollectionPreviewError("The open popover did not grow for the error message.")
            }
            let errorHeight = window.frame.height
            store.start()
            let recoveryDeadline = ContinuousClock.now + .seconds(10)
            while store.error != nil && .now < recoveryDeadline {
              try await Task.sleep(for: .milliseconds(50))
            }
            guard store.error == nil else { throw CollectionPreviewError("Recovery timed out.") }
            try await Task.sleep(for: .milliseconds(300))
            try await Self.capture(window, to: stem + "-recovered.png")
            guard window.frame.height < errorHeight else {
              throw CollectionPreviewError("The open popover did not shrink after recovery.")
            }
            print("Same-popover loading → data → error → recovery: passed")
          }
          MactopApplication.quit()
        } catch {
          await store.stop()
          fputs("Capture failed: \(error.localizedDescription)\n", stderr)
          exit(1)
        }
      }
    }

    private static func argument(_ name: String, in arguments: [String]) -> String? {
      guard let index = arguments.firstIndex(of: name), arguments.indices.contains(index + 1) else {
        return nil
      }
      return arguments[index + 1]
    }

    @available(macOS 14.4, *)
    private static func captureMenu(_ item: MetricStatusItem, to path: String) async throws {
      item.popover.performClose(nil)
      let visible = Set(NSApp.windows.filter(\.isVisible).map(\.windowNumber))
      let menu = item.makeMenu()
      let button = item.statusItem.button!
      let point = button.window!.convertPoint(toScreen: button.convert(.zero, to: nil))
      let close: @MainActor @Sendable () -> Void = { menu.cancelTracking() }
      let open: @MainActor @Sendable () -> Void = {
        _ = menu.popUp(positioning: nil, at: point, in: nil)
      }
      let timeout = Timer(timeInterval: 3, repeats: false) { _ in
        MainActor.assumeIsolated { close() }
      }
      RunLoop.main.add(timeout, forMode: .common)
      defer {
        timeout.invalidate()
        menu.cancelTracking()
      }
      RunLoop.main.perform {
        MainActor.assumeIsolated { open() }
      }
      try await Task.sleep(for: .milliseconds(300))
      guard
        let window = NSApp.windows.first(where: {
          $0.isVisible && !visible.contains($0.windowNumber)
        })
      else { throw CollectionPreviewError("The native menu window did not appear.") }
      try await capture(window, to: path)
    }

    @available(macOS 14.4, *)
    private static func capture(_ window: NSWindow, to path: String) async throws {
      let content = try await SCShareableContent.currentProcess
      guard let owned = content.windows.first(where: { $0.windowID == window.windowNumber }) else {
        throw CollectionPreviewError("The popover window is not capturable.")
      }
      let filter = SCContentFilter(desktopIndependentWindow: owned)
      let configuration = SCStreamConfiguration()
      configuration.width = Int(filter.contentRect.width * CGFloat(filter.pointPixelScale))
      configuration.height = Int(filter.contentRect.height * CGFloat(filter.pointPixelScale))
      configuration.showsCursor = false
      configuration.ignoreShadowsSingleWindow = true
      let image = try await SCScreenshotManager.captureImage(
        contentFilter: filter, configuration: configuration)
      guard let data = NSBitmapImageRep(cgImage: image).representation(using: .png, properties: [:])
      else {
        throw CollectionPreviewError("Could not encode the window image.")
      }
      try data.write(to: URL(fileURLWithPath: path))
    }
  }

  private struct CollectionPreviewError: LocalizedError {
    let message: String
    init(_ message: String) { self.message = message }
    var errorDescription: String? { message }
  }
#endif
