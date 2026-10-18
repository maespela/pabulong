import AppKit
import CoreText
import SwiftUI

// Pabulong's design language: the logo kit's navy and signal-blue palette,
// Montserrat for headings, DM Sans for everything else, rounded white cards on
// an off-white window, and simple line icons. Dark mode swaps in deep navy.

enum Brand {
    private static func dynamic(light: UInt32, dark: UInt32, lightAlpha: CGFloat = 1, darkAlpha: CGFloat = 1) -> Color {
        Color(nsColor: NSColor(name: nil) { appearance in
            let isDark = appearance.bestMatch(from: [.aqua, .darkAqua]) == .darkAqua
            return NSColor(hex: isDark ? dark : light, alpha: isDark ? darkAlpha : lightAlpha)
        })
    }

    static func nsColor(_ color: Color) -> NSColor { NSColor(color) }

    // Logo kit
    static let navy = Color(nsColor: NSColor(hex: 0x0D2C52))
    static let deepNavy = Color(nsColor: NSColor(hex: 0x0B1F3A))
    static let navy2 = Color(nsColor: NSColor(hex: 0x123B6D))
    static let signal = Color(nsColor: NSColor(hex: 0x1A3CFF))
    static let lavender = Color(nsColor: NSColor(hex: 0xC6C9E3))
    static let live = Color(nsColor: NSColor(hex: 0x22A55B))

    // Text
    static let text = dynamic(light: 0x0D2C52, dark: 0xEEF1F7)
    static let secondaryText = dynamic(light: 0x5B6472, dark: 0xA6AFC4)
    /// Signal blue where it is used as ink (icons, links). Lifted on navy so it stays legible.
    static let accentInk = dynamic(light: 0x1A3CFF, dark: 0x8C9DFF)

    // Surfaces
    static let window = dynamic(light: 0xECEEF1, dark: 0x0B1F3A)
    static let sidebar = dynamic(light: 0xE4E7EC, dark: 0x091930)
    static let sidebarSelection = dynamic(light: 0xDCE2EC, dark: 0x1A3A64)
    static let popover = dynamic(light: 0xF3F4F6, dark: 0x0D2442)
    static let card = dynamic(light: 0xFFFFFF, dark: 0x112B4D)
    static let cardStroke = dynamic(light: 0x000000, dark: 0xFFFFFF, lightAlpha: 0.06, darkAlpha: 0.07)
    static let rowDivider = dynamic(light: 0xE7E9ED, dark: 0xFFFFFF, darkAlpha: 0.08)
    static let separator = dynamic(light: 0xD5D9DF, dark: 0xFFFFFF, darkAlpha: 0.12)
    static let strip = dynamic(light: 0xE8EBF0, dark: 0x173761)
    static let control = dynamic(light: 0xFFFFFF, dark: 0x1A3A63)
    static let switchOff = dynamic(light: 0xC9CED6, dark: 0x2D4872)
    static let secondaryButton = dynamic(light: 0xDADDE3, dark: 0x1E3D66)
    static let hover = dynamic(light: 0x0D2C52, dark: 0xFFFFFF, lightAlpha: 0.06, darkAlpha: 0.08)
    static let meterTrack = dynamic(light: 0x0D2C52, dark: 0xFFFFFF, lightAlpha: 0.10, darkAlpha: 0.12)

    // Glass: light brand tints laid over a behind-window blur, translucent
    // enough that the window feels light without losing its colour.
    static let sidebarGlass = dynamic(light: 0xEEF0F4, dark: 0x091930, lightAlpha: 0.50, darkAlpha: 0.68)
    static let windowGlass = dynamic(light: 0xF5F6F8, dark: 0x0B1F3A, lightAlpha: 0.66, darkAlpha: 0.84)
    static let popoverGlass = dynamic(light: 0xF7F7F8, dark: 0x0D2442, lightAlpha: 0.62, darkAlpha: 0.80)
}

extension NSColor {
    convenience init(hex: UInt32, alpha: CGFloat = 1) {
        self.init(
            srgbRed: CGFloat((hex >> 16) & 0xFF) / 255,
            green: CGFloat((hex >> 8) & 0xFF) / 255,
            blue: CGFloat(hex & 0xFF) / 255,
            alpha: alpha
        )
    }
}

// MARK: - Fonts

/// Montserrat and DM Sans ship in Contents/Resources/Fonts (SIL OFL 1.1). Both
/// are variable fonts, so weights are set on the `wght` axis. When the files
/// are missing (e.g. `swift run`) every style falls back to the system font.
enum BrandFonts {
    nonisolated(unsafe) private static var montserrat: CTFontDescriptor?
    nonisolated(unsafe) private static var dmSans: CTFontDescriptor?

    private static let weightAxis = 0x7767_6874 as NSNumber // 'wght'
    private static let opticalSizeAxis = 0x6F70_737A as NSNumber // 'opsz'

    static func register() {
        montserrat = load("Montserrat-wght")
        dmSans = load("DMSans-wght")
    }

    private static func load(_ name: String) -> CTFontDescriptor? {
        let bundle = Bundle.main
        guard let url = bundle.url(forResource: name, withExtension: "ttf", subdirectory: "Fonts")
            ?? bundle.url(forResource: name, withExtension: "ttf") else { return nil }
        // Fails harmlessly if already registered; the descriptor below works either way.
        CTFontManagerRegisterFontsForURL(url as CFURL, .process, nil)
        let descriptors = CTFontManagerCreateFontDescriptorsFromURL(url as CFURL) as? [CTFontDescriptor]
        return descriptors?.first
    }

    static func display(_ size: CGFloat, _ weight: Font.Weight = .semibold) -> Font {
        font(montserrat, size: size, weight: weight, opticalSize: nil)
    }

    static func body(_ size: CGFloat, _ weight: Font.Weight = .regular) -> Font {
        // DM Sans' optical-size axis runs 9–40.
        font(dmSans, size: size, weight: weight, opticalSize: min(max(size, 9), 40))
    }

    static func bodyNSFont(_ size: CGFloat, _ weight: Font.Weight = .regular) -> NSFont {
        guard let base = dmSans else { return .systemFont(ofSize: size, weight: nsWeight(weight)) }
        return ctFont(base, size: size, weight: weight, opticalSize: min(max(size, 9), 40)) as NSFont
    }

    private static func font(_ base: CTFontDescriptor?, size: CGFloat, weight: Font.Weight, opticalSize: CGFloat?) -> Font {
        guard let base else { return .system(size: size, weight: weight) }
        return Font(ctFont(base, size: size, weight: weight, opticalSize: opticalSize))
    }

    private static func ctFont(_ base: CTFontDescriptor, size: CGFloat, weight: Font.Weight, opticalSize: CGFloat?) -> CTFont {
        var variation: [NSNumber: NSNumber] = [weightAxis: NSNumber(value: Double(wght(weight)))]
        if let opticalSize { variation[opticalSizeAxis] = NSNumber(value: Double(opticalSize)) }
        let attributes = [kCTFontVariationAttribute: variation] as CFDictionary
        let descriptor = CTFontDescriptorCreateCopyWithAttributes(base, attributes)
        return CTFontCreateWithFontDescriptor(descriptor, size, nil)
    }

    private static func wght(_ weight: Font.Weight) -> CGFloat {
        switch weight {
        case .ultraLight: 200
        case .thin: 100
        case .light: 300
        case .medium: 500
        case .semibold: 600
        case .bold: 700
        case .heavy: 800
        case .black: 900
        default: 400
        }
    }

    private static func nsWeight(_ weight: Font.Weight) -> NSFont.Weight {
        switch weight {
        case .medium: .medium
        case .semibold: .semibold
        case .bold: .bold
        default: .regular
        }
    }
}

// MARK: - Line icons

/// The launch video's line icons, drawn from their 24×24 SVG geometry.
enum BrandIcon {
    case calls, settings, diagnostics, window, power, record

    fileprivate func path(in rect: CGRect) -> Path {
        let s = min(rect.width, rect.height) / 24
        let ox = rect.midX - 12 * s
        let oy = rect.midY - 12 * s
        func p(_ x: CGFloat, _ y: CGFloat) -> CGPoint { CGPoint(x: ox + x * s, y: oy + y * s) }
        var path = Path()
        switch self {
        case .calls:
            for (y, end) in [(6.0, 20.0), (12.0, 20.0), (18.0, 14.0)] {
                path.move(to: p(4, y))
                path.addLine(to: p(end, y))
            }
        case .settings:
            path.addEllipse(in: CGRect(origin: p(8.8, 8.8), size: CGSize(width: 6.4 * s, height: 6.4 * s)))
            let rays: [(CGFloat, CGFloat, CGFloat, CGFloat)] = [
                (12, 2.8, 12, 5.8), (12, 18.2, 12, 21.2), (2.8, 12, 5.8, 12), (18.2, 12, 21.2, 12),
                (5.5, 5.5, 7.6, 7.6), (16.4, 16.4, 18.5, 18.5), (5.5, 18.5, 7.6, 16.4), (16.4, 7.6, 18.5, 5.5)
            ]
            for (x1, y1, x2, y2) in rays {
                path.move(to: p(x1, y1))
                path.addLine(to: p(x2, y2))
            }
        case .diagnostics:
            path.move(to: p(3, 12))
            for (x, y) in [(7.0, 12.0), (9.5, 6.0), (14.5, 18.0), (17.0, 12.0), (21.0, 12.0)] {
                path.addLine(to: p(x, y))
            }
        case .window:
            path.addRoundedRect(in: CGRect(origin: p(3, 5), size: CGSize(width: 18 * s, height: 14 * s)), cornerSize: CGSize(width: 2.5 * s, height: 2.5 * s))
            path.move(to: p(3, 9))
            path.addLine(to: p(21, 9))
        case .power:
            path.move(to: p(12, 3))
            path.addLine(to: p(12, 11))
            // Arc through the bottom from (17.5, 6.8) round to (6.5, 6.8), radius 7.5.
            let center = p(12, 6.8 + 5.099)
            path.move(to: p(17.5, 6.8))
            path.addArc(center: center, radius: 7.5 * s, startAngle: .degrees(-42.8), endAngle: .degrees(222.8), clockwise: false)
        case .record:
            path.addEllipse(in: CGRect(origin: p(3, 3), size: CGSize(width: 18 * s, height: 18 * s)))
        }
        return path
    }

    fileprivate var lineWidth: CGFloat {
        switch self {
        case .record: 2
        case .power: 1.9
        case .window: 1.8
        default: 1.9
        }
    }
}

struct BrandIconView: View {
    var icon: BrandIcon
    var size: CGFloat = 18

    var body: some View {
        Canvas { context, canvasSize in
            let rect = CGRect(origin: .zero, size: canvasSize)
            let scale = min(canvasSize.width, canvasSize.height) / 24
            let style = StrokeStyle(lineWidth: icon.lineWidth * scale, lineCap: .round, lineJoin: .round)
            context.stroke(icon.path(in: rect), with: .foreground, style: style)
            if icon == .record {
                let r = 4.6 * scale
                context.fill(Path(ellipseIn: CGRect(x: rect.midX - r, y: rect.midY - r, width: r * 2, height: r * 2)), with: .foreground)
            }
        }
        .frame(width: size, height: size)
        .accessibilityHidden(true)
    }
}

// MARK: - Controls

/// The rounded switch from the video: signal blue when on.
struct BrandSwitchStyle: ToggleStyle {
    func makeBody(configuration: Configuration) -> some View {
        BrandSwitchBody(configuration: configuration)
    }
}

private struct BrandSwitchBody: View {
    let configuration: ToggleStyleConfiguration
    @Environment(\.isEnabled) private var isEnabled

    var body: some View {
        let on = configuration.isOn
        ZStack(alignment: on ? .trailing : .leading) {
            Capsule()
                .fill(on ? Brand.signal : Brand.switchOff)
            Circle()
                .fill(Color.white)
                .shadow(color: .black.opacity(0.25), radius: 1.5, y: 1)
                .padding(2)
        }
        .frame(width: 38, height: 22)
        .opacity(isEnabled ? 1 : 0.45)
        .contentShape(Capsule())
        .onTapGesture {
            withAnimation(.snappy(duration: 0.16)) { configuration.isOn.toggle() }
        }
        .animation(.snappy(duration: 0.16), value: on)
    }
}

struct BrandSwitch: View {
    var title: String
    @Binding var isOn: Bool

    var body: some View {
        Toggle(title, isOn: $isOn)
            .toggleStyle(BrandSwitchStyle())
            .accessibilityLabel(title)
    }
}

/// White rounded pop-up button with the signal-blue arrow badge.
struct BrandPopUp<SelectionValue: Hashable, Content: View>: View {
    var title: String
    @Binding var selection: SelectionValue
    var valueLabel: String
    @ViewBuilder var content: () -> Content

    var body: some View {
        Menu {
            Picker(title, selection: $selection) {
                content()
            }
            .pickerStyle(.inline)
            .labelsHidden()
        } label: {
            HStack(spacing: 8) {
                Text(valueLabel)
                    .font(BrandFonts.body(13))
                    .foregroundStyle(Brand.text)
                    .lineLimit(1)
                Image(systemName: "chevron.up.chevron.down")
                    .font(.system(size: 8, weight: .bold))
                    .foregroundStyle(.white)
                    .frame(width: 16, height: 16)
                    .background(Brand.signal, in: RoundedRectangle(cornerRadius: 4, style: .continuous))
            }
            .padding(.leading, 10)
            .padding(.trailing, 5)
            .frame(height: 26)
            .background(Brand.control, in: RoundedRectangle(cornerRadius: 6, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 6, style: .continuous).strokeBorder(Brand.cardStroke))
            .shadow(color: .black.opacity(0.10), radius: 1, y: 0.5)
            .contentShape(Rectangle())
        }
        .menuStyle(.button)
        .buttonStyle(.plain)
        .menuIndicator(.hidden)
        .fixedSize()
        .accessibilityLabel(title)
        .accessibilityValue(valueLabel)
    }
}

struct BrandButtonStyle: ButtonStyle {
    enum Kind { case primary, secondary, destructive }
    var kind: Kind = .primary
    var fullWidth = false
    var height: CGFloat = 30

    func makeBody(configuration: Configuration) -> some View {
        BrandButtonBody(configuration: configuration, kind: kind, fullWidth: fullWidth, height: height)
    }
}

private struct BrandButtonBody: View {
    let configuration: ButtonStyleConfiguration
    let kind: BrandButtonStyle.Kind
    let fullWidth: Bool
    let height: CGFloat
    @Environment(\.isEnabled) private var isEnabled

    private var fill: Color {
        switch kind {
        case .primary: Brand.signal
        case .secondary: Brand.secondaryButton
        case .destructive: Color(nsColor: .systemRed)
        }
    }

    var body: some View {
        configuration.label
            .font(BrandFonts.body(13, .semibold))
            .labelStyle(BrandLabelStyle())
            .foregroundStyle(kind == .secondary ? Brand.text : Color.white)
            .padding(.horizontal, 14)
            .frame(maxWidth: fullWidth ? .infinity : nil, minHeight: height)
            .background(fill, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
            .opacity(isEnabled ? (configuration.isPressed ? 0.82 : 1) : 0.45)
            .contentShape(RoundedRectangle(cornerRadius: 8, style: .continuous))
    }
}

struct BrandLabelStyle: LabelStyle {
    func makeBody(configuration: Configuration) -> some View {
        HStack(spacing: 6) {
            configuration.icon
            configuration.title
        }
    }
}

/// Borderless row/icon button with a soft hover wash.
struct BrandPlainButtonStyle: ButtonStyle {
    var cornerRadius: CGFloat = 7

    func makeBody(configuration: Configuration) -> some View {
        BrandPlainButtonBody(configuration: configuration, cornerRadius: cornerRadius)
    }
}

private struct BrandPlainButtonBody: View {
    let configuration: ButtonStyleConfiguration
    let cornerRadius: CGFloat
    @Environment(\.isEnabled) private var isEnabled
    @StateObject private var hover = HoverState()

    var body: some View {
        configuration.label
            .background(
                RoundedRectangle(cornerRadius: cornerRadius, style: .continuous)
                    .fill(Brand.hover)
                    .opacity(isEnabled && (configuration.isPressed || hover.isHovering) ? 1 : 0)
            )
            .opacity(isEnabled ? 1 : 0.4)
            .contentShape(RoundedRectangle(cornerRadius: cornerRadius, style: .continuous))
            .onHover { hover.isHovering = $0 }
    }
}

@MainActor
final class HoverState: ObservableObject {
    @Published var isHovering = false
}

// MARK: - Cards

/// A titled group of rows on a white rounded card, as in the video's Settings.
struct BrandSection<Content: View>: View {
    var title: String?
    @ViewBuilder var content: () -> Content

    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if let title {
                Text(title)
                    .font(BrandFonts.body(12, .bold))
                    .foregroundStyle(Brand.secondaryText)
                    .padding(.leading, 4)
            }
            VStack(spacing: 0) {
                content()
            }
            .background(Brand.card, in: RoundedRectangle(cornerRadius: 10, style: .continuous))
            .overlay(RoundedRectangle(cornerRadius: 10, style: .continuous).strokeBorder(Brand.cardStroke))
        }
    }
}

/// One row of a `BrandSection`: label (and optional caption) left, control right.
struct BrandRow<Trailing: View>: View {
    var title: String
    var caption: String?
    var showsDivider = true
    @ViewBuilder var trailing: () -> Trailing

    var body: some View {
        VStack(spacing: 0) {
            HStack(alignment: .center, spacing: 20) {
                VStack(alignment: .leading, spacing: 3) {
                    Text(title)
                        .font(BrandFonts.body(14))
                        .foregroundStyle(Brand.text)
                    if let caption {
                        Text(caption)
                            .font(BrandFonts.body(11.5))
                            .foregroundStyle(Brand.secondaryText)
                            .fixedSize(horizontal: false, vertical: true)
                    }
                }
                .frame(maxWidth: .infinity, alignment: .leading)
                trailing()
            }
            .padding(.horizontal, 14)
            .padding(.vertical, caption == nil ? 9 : 10)
            .frame(minHeight: caption == nil ? 44 : 60)
            if showsDivider {
                Rectangle().fill(Brand.rowDivider).frame(height: 1)
            }
        }
    }
}

/// A free-form row inside a `BrandSection` (notes, link rows).
struct BrandCustomRow<Content: View>: View {
    var showsDivider = true
    @ViewBuilder var content: () -> Content

    var body: some View {
        VStack(spacing: 0) {
            content()
                .frame(maxWidth: .infinity, alignment: .leading)
                .padding(.horizontal, 14)
                .padding(.vertical, 10)
                .frame(minHeight: 44)
            if showsDivider {
                Rectangle().fill(Brand.rowDivider).frame(height: 1)
            }
        }
    }
}

// MARK: - Window chrome

/// Behind-window blur (works in a standard opaque window, like a system
/// sidebar). Lay a `Brand.*Glass` tint over it.
struct GlassBlur: NSViewRepresentable {
    var material: NSVisualEffectView.Material = .sidebar

    func makeNSView(context: Context) -> NSVisualEffectView {
        let view = NSVisualEffectView()
        view.material = material
        view.blendingMode = .behindWindow
        view.state = .followsWindowActiveState
        return view
    }

    func updateNSView(_ view: NSVisualEffectView, context: Context) {
        view.material = material
    }
}

// MARK: - Alerts

/// A Pabulong-styled modal alert: mark tile, bold title, message, and full-width
/// rounded buttons with a signal-blue primary. Returns the chosen button index.
@MainActor
enum BrandAlert {
    struct Choice {
        var title: String
        var kind: BrandButtonStyle.Kind
        var isCancel = false
    }

    private final class Panel: NSPanel {
        override var canBecomeKey: Bool { true }
        override var canBecomeMain: Bool { true }
    }

    static func run(title: String, message: String?, messageAlignment: TextAlignment = .center, choices: [Choice]) -> Int {
        let panel = Panel(
            contentRect: NSRect(x: 0, y: 0, width: 300, height: 200),
            styleMask: [.titled, .fullSizeContentView],
            backing: .buffered,
            defer: false
        )
        panel.titlebarAppearsTransparent = true
        panel.titleVisibility = .hidden
        panel.isMovableByWindowBackground = true
        panel.appearance = NSAppearance(named: .aqua)
        panel.backgroundColor = NSColor(Brand.popover)
        panel.title = title
        for button in [NSWindow.ButtonType.closeButton, .miniaturizeButton, .zoomButton] {
            panel.standardWindowButton(button)?.isHidden = true
        }

        var result = choices.firstIndex(where: \.isCancel) ?? (choices.count - 1)
        let view = BrandAlertView(title: title, message: message, messageAlignment: messageAlignment, choices: choices) { index in
            result = index
            NSApp.stopModal()
        }
        let host = NSHostingView(rootView: view)
        panel.contentView = host
        panel.setContentSize(host.fittingSize)
        panel.center()
        NSApp.activate(ignoringOtherApps: true)
        NSApp.runModal(for: panel)
        panel.orderOut(nil)
        return result
    }
}

private struct BrandAlertView: View {
    var title: String
    var message: String?
    var messageAlignment: TextAlignment
    var choices: [BrandAlert.Choice]
    var onChoose: (Int) -> Void

    var body: some View {
        VStack(spacing: 0) {
            Image(nsImage: PabulongMarkDrawing.image(height: 30, palette: .color))
                .frame(width: 52, height: 52)
                .background(Color.white, in: RoundedRectangle(cornerRadius: 12, style: .continuous))
                .shadow(color: .black.opacity(0.14), radius: 3, y: 1)
                .padding(.bottom, 12)
                .accessibilityHidden(true)

            Text(title)
                .font(BrandFonts.body(16, .bold))
                .foregroundStyle(Brand.text)
                .multilineTextAlignment(.center)
                .fixedSize(horizontal: false, vertical: true)
                .padding(.bottom, 8)

            if let message {
                Text(message)
                    .font(BrandFonts.body(12))
                    .lineSpacing(2)
                    .foregroundStyle(Brand.text.opacity(0.85))
                    .multilineTextAlignment(messageAlignment)
                    .frame(maxWidth: .infinity, alignment: messageAlignment == .leading ? .leading : .center)
                    .fixedSize(horizontal: false, vertical: true)
                    .padding(.bottom, 16)
            }

            VStack(spacing: 7) {
                ForEach(Array(choices.enumerated()), id: \.offset) { index, choice in
                    let button = Button(choice.title) { onChoose(index) }
                        .buttonStyle(BrandButtonStyle(kind: choice.kind, fullWidth: true, height: 30))
                    if index == 0 {
                        button.keyboardShortcut(.defaultAction)
                    } else if choice.isCancel {
                        button.keyboardShortcut(.cancelAction)
                    } else {
                        button
                    }
                }
            }
        }
        .padding(.top, 22)
        .padding(.horizontal, 18)
        .padding(.bottom, 16)
        .frame(width: 340)
        .background(Brand.popover)
    }
}
