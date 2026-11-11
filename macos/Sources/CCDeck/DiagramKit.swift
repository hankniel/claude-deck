import AppKit
import SwiftUI

// MARK: - colours

extension Color {
    /// "#rgb", "#rrggbb" or a few CSS names.
    init?(css: String?) {
        guard var s = css?.trimmingCharacters(in: .whitespaces).lowercased(), !s.isEmpty else { return nil }
        let names: [String: String] = ["red": "e5484d", "green": "30a46c", "blue": "3e63dd", "orange": "f76b15",
                                       "yellow": "ffc53d", "purple": "8e4ec6", "pink": "d6409f", "gray": "8b8d98",
                                       "grey": "8b8d98", "black": "111111", "white": "ffffff"]
        if let n = names[s] { s = "#" + n }
        guard s.hasPrefix("#") else { return nil }
        var h = String(s.dropFirst())
        if h.count == 3 { h = h.map { "\($0)\($0)" }.joined() }
        guard h.count >= 6, let v = UInt32(h.prefix(6), radix: 16) else { return nil }
        self.init(red: Double((v >> 16) & 0xff) / 255, green: Double((v >> 8) & 0xff) / 255, blue: Double(v & 0xff) / 255)
    }

    init(hex: UInt32) {
        self.init(red: Double((hex >> 16) & 0xff) / 255, green: Double((hex >> 8) & 0xff) / 255, blue: Double(hex & 0xff) / 255)
    }

    /// Rough luminance for picking a readable ink.
    var isLight: Bool {
        guard let c = NSColor(self).usingColorSpace(.sRGB) else { return true }
        return 0.2126 * c.redComponent + 0.7152 * c.greenComponent + 0.0722 * c.blueComponent > 0.6
    }
}

struct DiagramTheme {
    let dark: Bool
    var bg: Color { dark ? Color(hex: 0x1E1E1E) : Color(hex: 0xFBFAF8) }
    var fg: Color { dark ? Color(hex: 0xE8E6E3) : Color(hex: 0x1F1E1D) }
    var muted: Color { dark ? Color(hex: 0x9A968F) : Color(hex: 0x6F6B66) }
    var card: Color { dark ? Color(hex: 0x2A2A2A) : .white }
    var cardAlt: Color { dark ? Color(hex: 0x252525) : Color(hex: 0xF6F5F2) }
    var border: Color { dark ? Color(hex: 0x47443F) : Color(hex: 0xD9D6D0) }
    var edge: Color { dark ? Color(hex: 0x77736C) : Color(hex: 0x9C978F) }
    var accent: Color { DeckTheme.accent }
    var blue: Color { dark ? Color(hex: 0x4C9AE8) : Color(hex: 0x2272B4) }
    var blueTint: Color { dark ? Color(hex: 0x1D3247) : Color(hex: 0xE8F1FA) }
    var group: Color { dark ? Color(hex: 0x262626) : Color(hex: 0xF3F1ED) }
}

// MARK: - drawing helpers

enum Draw {
    static func text(_ ctx: inout GraphicsContext, _ s: String, at p: CGPoint, anchor: UnitPoint = .topLeading,
                     size: CGFloat = 13, weight: Font.Weight = .regular, mono: Bool = false, color: Color) {
        let font: Font = mono ? .system(size: size, design: .monospaced) : .system(size: size, weight: weight)
        ctx.draw(Text(s).font(font).foregroundStyle(color), at: p, anchor: anchor)
    }

    /// Text clipped to a box (one line, truncated with an ellipsis).
    static func text(_ ctx: inout GraphicsContext, _ s: String, in r: CGRect, alignment: Alignment = .leading,
                     size: CGFloat = 13, weight: Font.Weight = .regular, mono: Bool = false, color: Color) {
        let font: Font = mono ? .system(size: size, design: .monospaced) : .system(size: size, weight: weight)
        let resolved = ctx.resolve(Text(s).font(font))
        let measured = resolved.measure(in: CGSize(width: 10_000, height: r.height))
        var str = s
        if measured.width > r.width, r.width > 12 {
            // trim until it fits
            var lo = 0, hi = s.count
            while lo < hi {
                let mid = (lo + hi + 1) / 2
                let cand = String(s.prefix(mid)) + "…"
                if ctx.resolve(Text(cand).font(font)).measure(in: CGSize(width: 10_000, height: r.height)).width <= r.width { lo = mid } else { hi = mid - 1 }
            }
            str = String(s.prefix(lo)) + "…"
        }
        let t = Text(str).font(font).foregroundStyle(color)
        let anchor: UnitPoint
        let p: CGPoint
        switch alignment {
        case .trailing: anchor = .trailing; p = CGPoint(x: r.maxX, y: r.midY)
        case .center: anchor = .center; p = CGPoint(x: r.midX, y: r.midY)
        default: anchor = .leading; p = CGPoint(x: r.minX, y: r.midY)
        }
        ctx.draw(t, at: p, anchor: anchor)
    }

    static func arrowHead(_ ctx: inout GraphicsContext, tip: CGPoint, from: CGPoint, size: CGFloat = 8, color: Color) {
        let a = atan2(tip.y - from.y, tip.x - from.x)
        var p = Path()
        p.move(to: tip)
        p.addLine(to: CGPoint(x: tip.x + size * cos(a + .pi - 0.45), y: tip.y + size * sin(a + .pi - 0.45)))
        p.addLine(to: CGPoint(x: tip.x + size * cos(a + .pi + 0.45), y: tip.y + size * sin(a + .pi + 0.45)))
        p.closeSubpath()
        ctx.fill(p, with: .color(color))
    }

    /// Where the line from the rect's centre toward `p` leaves it.
    static func boundary(_ r: CGRect, toward p: CGPoint) -> CGPoint {
        let dx = p.x - r.midX, dy = p.y - r.midY
        if abs(dx) < 1e-9 && abs(dy) < 1e-9 { return CGPoint(x: r.midX, y: r.midY) }
        let sx = abs(dx) > 1e-9 ? (r.width / 2) / abs(dx) : .infinity
        let sy = abs(dy) > 1e-9 ? (r.height / 2) / abs(dy) : .infinity
        let s = min(sx, sy)
        return CGPoint(x: r.midX + dx * s, y: r.midY + dy * s)
    }

    /// A smooth curve through points (Catmull-Rom).
    static func smooth(_ pts: [CGPoint]) -> Path {
        var p = Path()
        guard pts.count >= 2 else { return p }
        p.move(to: pts[0])
        if pts.count == 2 { p.addLine(to: pts[1]); return p }
        for i in 0..<(pts.count - 1) {
            let p0 = i == 0 ? pts[0] : pts[i - 1], p1 = pts[i], p2 = pts[i + 1]
            let p3 = i + 2 < pts.count ? pts[i + 2] : p2
            p.addCurve(to: p2,
                       control1: CGPoint(x: p1.x + (p2.x - p0.x) / 6, y: p1.y + (p2.y - p0.y) / 6),
                       control2: CGPoint(x: p2.x - (p3.x - p1.x) / 6, y: p2.y - (p3.y - p1.y) / 6))
        }
        return p
    }

    /// Horizontal S-curve between side anchors.
    static func sCurve(_ a: CGPoint, _ b: CGPoint, out: CGFloat = 1, in inDir: CGFloat = 1) -> Path {
        let d = max(abs(b.x - a.x) * 0.5, 40)
        var p = Path()
        p.move(to: a)
        p.addCurve(to: b, control1: CGPoint(x: a.x + d * out, y: a.y), control2: CGPoint(x: b.x - d * inDir, y: b.y))
        return p
    }

    static func sCurveDistance(_ a: CGPoint, _ b: CGPoint, out: CGFloat, in inDir: CGFloat, to q: CGPoint) -> CGFloat {
        let d = max(abs(b.x - a.x) * 0.5, 40)
        let c1 = CGPoint(x: a.x + d * out, y: a.y), c2 = CGPoint(x: b.x - d * inDir, y: b.y)
        var best = CGFloat.infinity
        for i in 0...40 {
            let t = CGFloat(i) / 40, u = 1 - t
            let x = u * u * u * a.x + 3 * u * u * t * c1.x + 3 * u * t * t * c2.x + t * t * t * b.x
            let y = u * u * u * a.y + 3 * u * u * t * c1.y + 3 * u * t * t * c2.y + t * t * t * b.y
            best = min(best, hypot(x - q.x, y - q.y))
        }
        return best
    }

    static func pill(_ ctx: inout GraphicsContext, _ s: String, center: CGPoint, size: CGFloat = 11, fg: Color, bg: Color) {
        let resolved = ctx.resolve(Text(s).font(.system(size: size)))
        let m = resolved.measure(in: CGSize(width: 1000, height: 100))
        let r = CGRect(x: center.x - m.width / 2 - 5, y: center.y - m.height / 2 - 1, width: m.width + 10, height: m.height + 2)
        ctx.fill(Path(roundedRect: r, cornerRadius: r.height / 2), with: .color(bg))
        ctx.draw(Text(s).font(.system(size: size)).foregroundStyle(fg), at: center, anchor: .center)
    }
}

// MARK: - pan / zoom surface

/// What a diagram does with the pointer (all points in content coordinates).
struct DiagramEvents {
    var hover: (CGPoint?) -> Void = { _ in }
    var tap: (CGPoint, Int, NSEvent.ModifierFlags) -> Void = { _, _, _ in }
    /// Start dragging an item at the point (true) instead of panning.
    var grab: (CGPoint) -> Bool = { _ in false }
    var drag: (CGSize) -> Void = { _ in }
    var drop: () -> Void = {}
}

/// Pannable (drag, scroll), zoomable (pinch, ⌘-scroll) drawing surface. `fitToken` changes refit.
struct ZoomCanvas: View {
    let contentSize: CGSize
    let minFit: CGFloat
    let anchor: CGPoint?
    let fitToken: Int
    let events: DiagramEvents
    let draw: (inout GraphicsContext, DiagramTheme) -> Void

    init(contentSize: CGSize, minFit: CGFloat = 0.15, anchor: CGPoint? = nil, fitToken: Int = 0,
         events: DiagramEvents = DiagramEvents(), draw: @escaping (inout GraphicsContext, DiagramTheme) -> Void) {
        self.contentSize = contentSize
        self.minFit = minFit
        self.anchor = anchor
        self.fitToken = fitToken
        self.events = events
        self.draw = draw
    }

    @Environment(\.colorScheme) var scheme
    @State var scale: CGFloat = 1
    @State var offset: CGSize = .zero
    @State var fitted = false
    @State var viewSize: CGSize = .zero
    @State var dragBase: CGSize? = nil
    @State var itemDrag = false
    @State var lastTranslation: CGSize = .zero
    @State var pinchBase: CGFloat? = nil

    private func toContent(_ p: CGPoint) -> CGPoint {
        CGPoint(x: (p.x - offset.width) / scale, y: (p.y - offset.height) / scale)
    }

    private func fit(_ size: CGSize) {
        guard size.width > 0, size.height > 0 else { return }
        let cw = max(contentSize.width, 1), ch = max(contentSize.height, 1)
        let f = min((size.width - 60) / cw, (size.height - 80) / ch)
        let s = min(max(f, min(minFit, 1)), 1.4)
        scale = s
        if s > f + 1e-6 {
            let a = anchor ?? CGPoint(x: (size.width / 2 - 30) / s, y: ch / 2)
            offset = CGSize(width: size.width / 2 - a.x * s, height: size.height / 2 - a.y * s)
        } else {
            offset = CGSize(width: (size.width - cw * s) / 2, height: max((size.height - ch * s) / 2, 16))
        }
        fitted = true
    }

    private func zoom(_ f: CGFloat, at p: CGPoint) {
        let s1 = min(max(scale * f, 0.08), 4)
        let k = s1 / scale
        offset = CGSize(width: p.x - (p.x - offset.width) * k, height: p.y - (p.y - offset.height) * k)
        scale = s1
    }

    var body: some View {
        let theme = DiagramTheme(dark: scheme == .dark)
        GeometryReader { geo in
            Canvas { ctx, size in
                ctx.fill(Path(CGRect(origin: .zero, size: size)), with: .color(theme.bg))
                let step = 22 * scale
                if step > 6 {
                    var dots = Path()
                    let ox = offset.width.truncatingRemainder(dividingBy: step)
                    let oy = offset.height.truncatingRemainder(dividingBy: step)
                    var y = oy < 0 ? oy + step : oy
                    while y < size.height {
                        var x = ox < 0 ? ox + step : ox
                        while x < size.width { dots.addRect(CGRect(x: x, y: y, width: 1.2, height: 1.2)); x += step }
                        y += step
                    }
                    ctx.fill(dots, with: .color(theme.fg.opacity(theme.dark ? 0.07 : 0.09)))
                }
                var inner = ctx
                inner.translateBy(x: offset.width, y: offset.height)
                inner.scaleBy(x: scale, y: scale)
                draw(&inner, theme)
            }
            .contentShape(Rectangle())
            .gesture(
                DragGesture(minimumDistance: 0, coordinateSpace: .local)
                    .onChanged { v in
                        if dragBase == nil {
                            dragBase = offset
                            lastTranslation = .zero
                            itemDrag = events.grab(toContent(v.startLocation))
                        }
                        if itemDrag {
                            let d = CGSize(width: (v.translation.width - lastTranslation.width) / scale,
                                           height: (v.translation.height - lastTranslation.height) / scale)
                            lastTranslation = v.translation
                            events.drag(d)
                        } else if let b = dragBase {
                            offset = CGSize(width: b.width + v.translation.width, height: b.height + v.translation.height)
                        }
                    }
                    .onEnded { v in
                        let moved = abs(v.translation.width) + abs(v.translation.height) > 3
                        if itemDrag { events.drop() }
                        if !moved {
                            let clicks = NSApp.currentEvent?.clickCount ?? 1
                            events.tap(toContent(v.location), clicks, NSEvent.modifierFlags)
                        }
                        dragBase = nil
                        itemDrag = false
                    }
            )
            .simultaneousGesture(
                MagnifyGesture()
                    .onChanged { v in
                        let base = pinchBase ?? scale
                        if pinchBase == nil { pinchBase = scale }
                        let target = min(max(base * v.magnification, 0.08), 4)
                        zoom(target / scale, at: v.startLocation)
                    }
                    .onEnded { _ in pinchBase = nil }
            )
            .onContinuousHover { phase in
                switch phase {
                case .active(let p): events.hover(toContent(p))
                case .ended: events.hover(nil)
                }
            }
            .background(ScrollMonitor { e, p in
                if e.modifierFlags.contains(.command) {
                    let dy = e.hasPreciseScrollingDeltas ? e.scrollingDeltaY * 0.01 : e.scrollingDeltaY * 0.1
                    zoom(min(max(1 + dy, 0.8), 1.25), at: p)
                } else {
                    let k: CGFloat = e.hasPreciseScrollingDeltas ? 1 : 12
                    offset = CGSize(width: offset.width + e.scrollingDeltaX * k, height: offset.height + e.scrollingDeltaY * k)
                }
                return true
            })
            .onAppear {
                viewSize = geo.size
                if !fitted { fit(geo.size) }
            }
            .onChange(of: geo.size) { _, s in
                viewSize = s
                if !fitted { fit(s) }
            }
            .onChange(of: fitToken) { _, _ in fit(viewSize) }
            .overlay(alignment: .bottomTrailing) {
                HStack(spacing: 0) {
                    Button { zoom(0.8, at: CGPoint(x: viewSize.width / 2, y: viewSize.height / 2)) } label: { Image(systemName: "minus.magnifyingglass") }
                    Button { fit(viewSize) } label: { Image(systemName: "arrow.up.left.and.down.right.magnifyingglass") }
                    Button { zoom(1.25, at: CGPoint(x: viewSize.width / 2, y: viewSize.height / 2)) } label: { Image(systemName: "plus.magnifyingglass") }
                }
                .buttonStyle(.borderless)
                .padding(.horizontal, 10).padding(.vertical, 6)
                .glassEffect(.regular, in: .capsule)
                .padding(14)
            }
        }
    }
}

/// Scroll-wheel events over a view, delivered with the point in the view (top-left origin).
struct ScrollMonitor: NSViewRepresentable {
    let onScroll: (NSEvent, CGPoint) -> Bool

    func makeNSView(context: Context) -> MonitorView {
        let v = MonitorView()
        v.onScroll = onScroll
        return v
    }

    func updateNSView(_ v: MonitorView, context: Context) { v.onScroll = onScroll }

    final class MonitorView: NSView {
        var onScroll: ((NSEvent, CGPoint) -> Bool)?
        private var monitor: Any?
        override var isFlipped: Bool { true }

        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            if let m = monitor { NSEvent.removeMonitor(m); monitor = nil }
            guard window != nil else { return }
            monitor = NSEvent.addLocalMonitorForEvents(matching: .scrollWheel) { [weak self] e in
                guard let self, let w = self.window, e.window === w else { return e }
                let p = self.convert(e.locationInWindow, from: nil)
                guard self.bounds.contains(p), !self.isHidden else { return e }
                return self.onScroll?(e, p) == true ? nil : e
            }
        }

        override func hitTest(_ point: NSPoint) -> NSView? { nil }
    }
}
