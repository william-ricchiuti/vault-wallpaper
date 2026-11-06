//! Tiles PNGs into a grid, downscaled, for comparing renders side by side.
//!   cargo run --release -p vw-render --example tile_png -- <out.png> <columns> <scale-down> <in.png>...
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    let (out, cols, k): (&str, usize, usize) = (&a[0], a[1].parse().unwrap(), a[2].parse().unwrap());
    let imgs: Vec<(u32, u32, Vec<u8>)> = a[3..]
        .iter()
        .map(|p| {
            let dec = png::Decoder::new(std::io::BufReader::new(std::fs::File::open(p).unwrap()));
            let mut r = dec.read_info().unwrap();
            let mut buf = vec![0; r.output_buffer_size().unwrap()];
            let info = r.next_frame(&mut buf).unwrap();
            let ch = info.color_type.samples();
            let rgb: Vec<u8> = buf[..info.buffer_size()].chunks(ch).flat_map(|px| [px[0], px[1], px[2]]).collect();
            (info.width, info.height, rgb)
        })
        .collect();
    let (tw, th) = (imgs[0].0 as usize / k, imgs[0].1 as usize / k);
    let rows = imgs.len().div_ceil(cols);
    let (w, h) = (tw * cols + (cols - 1) * 4, th * rows + (rows - 1) * 4);
    let mut canvas = vec![255u8; w * h * 3];
    for (n, (iw, _, px)) in imgs.iter().enumerate() {
        let (ox, oy) = ((n % cols) * (tw + 4), (n / cols) * (th + 4));
        for y in 0..th {
            for x in 0..tw {
                // Box filter over k x k source pixels.
                let mut acc = [0u32; 3];
                for dy in 0..k {
                    for dx in 0..k {
                        let i = ((y * k + dy) * *iw as usize + x * k + dx) * 3;
                        for c in 0..3 {
                            acc[c] += u32::from(px[i + c]);
                        }
                    }
                }
                let o = ((oy + y) * w + ox + x) * 3;
                for c in 0..3 {
                    canvas[o + c] = (acc[c] / (k * k) as u32) as u8;
                }
            }
        }
    }
    let f = std::io::BufWriter::new(std::fs::File::create(out).unwrap());
    let mut enc = png::Encoder::new(f, w as u32, h as u32);
    enc.set_color(png::ColorType::Rgb);
    enc.write_header().unwrap().write_image_data(&canvas).unwrap();
}
