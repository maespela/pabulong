#!/usr/bin/env swift
// Builds Pabulong.icns from the logo kit's app-icon master: the "Hush p" mark on
// a white tile. The master is a full-bleed square for iOS/Android; on macOS the
// tile sits inside Apple's standard icon grid (an 824 px rounded square with a
// soft shadow on a 1024 px canvas), so it matches other Mac app icons.
import AppKit
import Foundation

let root = URL(fileURLWithPath: FileManager.default.currentDirectoryPath)
let resources = root.appendingPathComponent("Sources/NoiselessApp/Resources")
let iconset = resources.appendingPathComponent("Pabulong.iconset")
let output = resources.appendingPathComponent("Pabulong.icns")

try FileManager.default.createDirectory(at: resources, withIntermediateDirectories: true)
try? FileManager.default.removeItem(at: iconset)
try FileManager.default.createDirectory(at: iconset, withIntermediateDirectories: true)

let icons: [(pixels: Int, name: String)] = [
    (16, "icon_16x16.png"), (32, "icon_16x16@2x.png"),
    (32, "icon_32x32.png"), (64, "icon_32x32@2x.png"),
    (128, "icon_128x128.png"), (256, "icon_128x128@2x.png"),
    (256, "icon_256x256.png"), (512, "icon_256x256@2x.png"),
    (512, "icon_512x512.png"), (1024, "icon_512x512@2x.png"),
]

// Brand colours (logo kit README).
let navy = CGColor(red: 13 / 255, green: 44 / 255, blue: 82 / 255, alpha: 1)
let signalBlue = CGColor(red: 26 / 255, green: 60 / 255, blue: 255 / 255, alpha: 1)

/// Draws the mark in its SVG coordinate space (y down). `smallSize` drops the
/// faded outer ring, as the kit does for its 16 px favicon.
func drawMark(_ ctx: CGContext, smallSize: Bool) {
    ctx.setFillColor(navy)
    ctx.addPath(CGPath(roundedRect: CGRect(x: 10, y: 14, width: 7, height: 44), cornerWidth: 3.5, cornerHeight: 3.5, transform: nil))
    ctx.fillPath()

    ctx.setStrokeColor(navy)
    ctx.setLineWidth(7)
    ctx.strokeEllipse(in: CGRect(x: 25.5 - 12, y: 26 - 12, width: 24, height: 24))

    ctx.setLineCap(.round)
    ctx.setLineWidth(4.5)
    for (radius, alpha) in [(20.0, 1.0), (27.0, 0.4)] where !(smallSize && radius == 27) {
        ctx.setStrokeColor(signalBlue.copy(alpha: alpha)!)
        // In y-down coordinates this sweeps through 0° — the arc on the right.
        ctx.addArc(center: CGPoint(x: 25.5, y: 26), radius: radius, startAngle: -40 * .pi / 180, endAngle: 40 * .pi / 180, clockwise: false)
        ctx.strokePath()
    }
}

func drawIcon(pixels: Int) throws -> Data {
    guard let bitmap = NSBitmapImageRep(
        bitmapDataPlanes: nil, pixelsWide: pixels, pixelsHigh: pixels,
        bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
        colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0
    ), let ns = NSGraphicsContext(bitmapImageRep: bitmap) else {
        throw NSError(domain: "icon", code: 1)
    }
    let ctx = ns.cgContext
    let s = CGFloat(pixels) / 1024
    // Work in 1024-unit, y-down coordinates.
    ctx.translateBy(x: 0, y: CGFloat(pixels))
    ctx.scaleBy(x: s, y: -s)
    ctx.clear(CGRect(x: 0, y: 0, width: 1024, height: 1024))

    let tile = CGRect(x: 100, y: 100, width: 824, height: 824)
    let tilePath = CGPath(roundedRect: tile, cornerWidth: 185.4, cornerHeight: 185.4, transform: nil)
    ctx.saveGState()
    if pixels >= 64 {
        ctx.setShadow(offset: CGSize(width: 0, height: 10), blur: 28, color: CGColor(gray: 0, alpha: 0.28))
    }
    ctx.setFillColor(CGColor(gray: 1, alpha: 1))
    ctx.addPath(tilePath)
    ctx.fillPath()
    ctx.restoreGState()

    // The master places the mark at translate(190.793 192.555) scale(9.921462)
    // on a 1024 tile; map that tile onto the 824 px macOS tile.
    let k: CGFloat = 824 / 1024
    ctx.saveGState()
    ctx.addPath(tilePath)
    ctx.clip()
    ctx.translateBy(x: 100 + 190.793 * k, y: 100 + 192.555 * k)
    ctx.scaleBy(x: 9.921462 * k, y: 9.921462 * k)
    drawMark(ctx, smallSize: pixels <= 32)
    ctx.restoreGState()

    guard let png = bitmap.representation(using: .png, properties: [:]) else {
        throw NSError(domain: "icon", code: 2)
    }
    return png
}

for icon in icons {
    try drawIcon(pixels: icon.pixels).write(to: iconset.appendingPathComponent(icon.name))
}

let process = Process()
process.executableURL = URL(fileURLWithPath: "/usr/bin/iconutil")
process.arguments = ["-c", "icns", iconset.path, "-o", output.path]
try process.run()
process.waitUntilExit()
guard process.terminationStatus == 0 else {
    throw NSError(domain: "icon", code: Int(process.terminationStatus))
}
try? FileManager.default.removeItem(at: iconset)
print(output.path)
