import AppKit
import SwiftUI

/// The Pabulong "Hush p" mark, drawn from the logo kit's SVG geometry
/// (viewBox 7.94 4.33 48.88 55.73) so it stays sharp at every size.
enum PabulongMarkDrawing {
    static let viewBox = CGRect(x: 7.94, y: 4.33, width: 48.88, height: 55.73)

    enum Palette {
        case color      // navy p, signal-blue rings — light backgrounds
        case reversed   // white p, lavender rings — dark backgrounds
        case template   // black, for menu bar template images

        var stem: CGColor {
            switch self {
            case .color: CGColor(red: 13 / 255, green: 44 / 255, blue: 82 / 255, alpha: 1)
            case .reversed: CGColor(gray: 1, alpha: 1)
            case .template: CGColor(gray: 0, alpha: 1)
            }
        }

        var ring: CGColor {
            switch self {
            case .color: CGColor(red: 26 / 255, green: 60 / 255, blue: 1, alpha: 1)
            case .reversed: CGColor(red: 198 / 255, green: 201 / 255, blue: 227 / 255, alpha: 1)
            case .template: CGColor(gray: 0, alpha: 1)
            }
        }

        /// Outer ring opacity (40%, or 45% on dark backgrounds, per the kit).
        var outerRingAlpha: CGFloat { self == .reversed ? 0.45 : 0.4 }
    }

    /// Draws into `ctx` in SVG coordinates (y down). Rings are the "sound";
    /// hiding them shows Pabulong as off.
    static func draw(_ ctx: CGContext, palette: Palette, showRings: Bool, showOuterRing: Bool) {
        ctx.setFillColor(palette.stem)
        ctx.addPath(CGPath(roundedRect: CGRect(x: 10, y: 14, width: 7, height: 44), cornerWidth: 3.5, cornerHeight: 3.5, transform: nil))
        ctx.fillPath()

        ctx.setStrokeColor(palette.stem)
        ctx.setLineWidth(7)
        ctx.strokeEllipse(in: CGRect(x: 13.5, y: 14, width: 24, height: 24))

        guard showRings else { return }
        ctx.setLineCap(.round)
        ctx.setLineWidth(4.5)
        for (radius, alpha) in [(20.0, 1.0), (27.0, palette.outerRingAlpha)] where showOuterRing || radius == 20 {
            ctx.setStrokeColor(palette.ring.copy(alpha: alpha) ?? palette.ring)
            ctx.addArc(center: CGPoint(x: 25.5, y: 26), radius: radius, startAngle: -40 * .pi / 180, endAngle: 40 * .pi / 180, clockwise: false)
            ctx.strokePath()
        }
    }

    static func image(height: CGFloat, palette: Palette, showRings: Bool = true, alpha: CGFloat = 1) -> NSImage {
        let scale = height / viewBox.height
        let size = NSSize(width: ceil(viewBox.width * scale), height: height)
        let image = NSImage(size: size, flipped: true) { _ in
            guard let ctx = NSGraphicsContext.current?.cgContext else { return false }
            ctx.setAlpha(alpha)
            ctx.scaleBy(x: scale, y: scale)
            ctx.translateBy(x: -viewBox.minX, y: -viewBox.minY)
            // The faded outer ring turns into a smudge at tiny sizes (as the kit
            // does for its 16 px favicon).
            draw(ctx, palette: palette, showRings: showRings, showOuterRing: height >= 18)
            return true
        }
        image.isTemplate = palette == .template
        return image
    }
}

/// Full-colour mark for the app window and popover.
struct PabulongAppMark: View {
    var enabled: Bool
    var size: CGFloat
    @Environment(\.colorScheme) private var colorScheme

    var body: some View {
        Image(nsImage: PabulongMarkDrawing.image(
            height: size,
            palette: colorScheme == .dark ? .reversed : .color,
            showRings: enabled,
            alpha: enabled ? 1 : 0.55
        ))
        .frame(width: max(size + 4, 24), height: max(size + 4, 24))
        .accessibilityHidden(true)
    }
}

/// Menu bar icon: a template image so macOS tints it for light and dark menu bars.
struct MenuBarStatusMark: View {
    var enabled: Bool

    var body: some View {
        Image(nsImage: PabulongMarkDrawing.image(height: 19, palette: .template, showRings: enabled, alpha: enabled ? 1 : 0.6))
            .accessibilityHidden(true)
    }
}
