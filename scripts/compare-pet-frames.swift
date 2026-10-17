// Static image QA only: compare the exported before/after sprite poses.
import AppKit
import Foundation

func pixels(_ url: URL) throws -> (Int, Int, [UInt8]) {
    guard let image = NSImage(contentsOf:url), let cg = image.cgImage(forProposedRect:nil,context:nil,hints:nil) else { throw NSError(domain:"image",code:1) }
    var bytes = [UInt8](repeating:0,count:cg.width*cg.height*4)
    bytes.withUnsafeMutableBytes { buffer in
        let context = CGContext(data:buffer.baseAddress,width:cg.width,height:cg.height,bitsPerComponent:8,bytesPerRow:cg.width*4,space:CGColorSpaceCreateDeviceRGB(),bitmapInfo:CGImageAlphaInfo.premultipliedLast.rawValue)!
        context.draw(cg,in:CGRect(x:0,y:0,width:cg.width,height:cg.height))
    }
    return (cg.width,cg.height,bytes)
}
@main struct ComparePetFrames {
    static func main() throws {
        guard CommandLine.arguments.count==4 else { throw NSError(domain:"Expected before, after and report paths",code:1) }
        let before=URL(fileURLWithPath:CommandLine.arguments[1]),after=URL(fileURLWithPath:CommandLine.arguments[2])
        var rows=[[String:Any]]()
        for name in try FileManager.default.contentsOfDirectory(atPath:before.path).filter({$0.hasSuffix(".png")}).sorted() {
            let a=try pixels(before.appendingPathComponent(name)),b=try pixels(after.appendingPathComponent(name))
            guard a.0==b.0 && a.1==b.1 else { throw NSError(domain:"Image dimensions changed: "+name,code:1) }
            var total=0,changed=0
            for pixel in 0..<(a.0*a.1) {
                var large=false
                for channel in 0..<3 { let difference=abs(Int(a.2[pixel*4+channel])-Int(b.2[pixel*4+channel]));total+=difference;large = large || difference>16 }
                if large { changed+=1 }
            }
            rows.append(["name":name,"meanAbsoluteRGB":Double(total)/Double(a.0*a.1*3),"fractionPixelsOver16":Double(changed)/Double(a.0*a.1)])
        }
        let mean=rows.reduce(0.0){$0+($1["meanAbsoluteRGB"] as! Double)}/Double(max(1,rows.count))
        let report:[String:Any] = ["images":rows.count,"meanAbsoluteRGB":mean,"rows":rows,"note":"Static first poses on light/dark backdrops at five scales; different compositor antialiasing can change edge pixels. Animation timing is verified separately."]
        try JSONSerialization.data(withJSONObject:report,options:[.prettyPrinted,.sortedKeys]).write(to:URL(fileURLWithPath:CommandLine.arguments[3]))
        print("Compared \(rows.count) pet pose images; mean absolute RGB difference \(mean) / 255")
    }
}
