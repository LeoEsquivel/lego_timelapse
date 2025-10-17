use std::path::PathBuf;
use std::fs;
use clap::{Parser};
use ffmpeg_next::{
    format,
    frame,
    util::rational::Rational,
    Packet,
    Error,
};
use image::{GenericImageView};

// Crea el video timelapse desde una carpeta con imagenes
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(required = true)]
    carpeta: PathBuf,

    #[arg(short, long, default_value_t = 10)]
    fps: u32,

    #[arg(short, long, default_value = "timelapse.mp4")]
    salida: String,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    ffmpeg_next::init()?;

    // Leer rutas de imágenes
    let mut paths: Vec<_> = fs::read_dir(&args.carpeta)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().map_or(false, |ext| ext == "jpg" || ext == "png"))
        .collect();
    paths.sort();

    if paths.is_empty() {
        println!("No se encontraron imágenes en la carpeta");
        return Ok(());
    }

    // Dimensiones de la primera imagen
    let (width, height) = get_image_dimensions(&paths[0])?;
    
    // Crear contexto de salida
    let mut octx = format::output(&args.salida)?;
    let codec_id = ffmpeg_next::codec::Id::H264;
    let stream = octx.add_stream(codec_id)?;
    let mut encoder = stream.codec().encoder().video()?;

    encoder.set_width(width);
    encoder.set_height(height);
    encoder.set_format(ffmpeg_next::format::Pixel::YUV420P);
    encoder.set_time_base(Rational::new(1, args.fps as i32));
    let mut encoder = encoder.open_as(codec_id)?;
    octx.write_header()?;

    // Procesar imágenes
    for (i, path) in paths.iter().enumerate() {
        println!("Procesando: {}", path.display());
        let mut img = image::ImageReader::open(path)?.decode()?;

        if img.width() != width || img.height() != height {
            println!("  Redimensionando de {}x{} a {}x{}", img.width(), img.height(), width, height);
            img = img.resize_exact(width, height, image::imageops::FilterType::Lanczos3);
        }

        let img = img.to_rgb8();
        // Convertir a frame RGB24
        let mut f = frame::Video::new(ffmpeg_next::format::Pixel::YUV420P, width, height);
        rgb_to_yuv420p(&img, &mut f, width, height);

        f.set_pts(Some(i as i64));
        encoder.send_frame(&f)?;

        receive_and_write_packets(&mut encoder, &mut octx)?;
    }

    // Vaciar encoder
    encoder.send_eof()?;
    receive_and_write_packets(&mut encoder, &mut octx)?;

    octx.write_trailer()?;
    println!("\nVideo timelapse guardado como '{}' exitosamente", args.salida);
    Ok(())
}

fn get_image_dimensions(path: &PathBuf) -> Result<(u32, u32), image::ImageError> {
    let img = image::ImageReader::open(path)?.decode()?;
    Ok(img.dimensions())
}

fn receive_and_write_packets(
    encoder: &mut ffmpeg_next::codec::encoder::Video,
    octx: &mut format::context::Output
) -> Result<(), Error> {
    let mut packet = Packet::empty();
    loop {
        match encoder.receive_packet(&mut packet) {
            Ok(_) => {
                packet.write_interleaved(octx)?;
            }
            Err(Error::Other { errno }) if errno == 11 || errno == ffmpeg_next::sys::AVERROR(ffmpeg_next::sys::EAGAIN) => {
                // El encoder necesita más frames antes de producir paquetes
                break;
            }
            Err(Error::Eof) => break,
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

fn rgb_to_yuv420p(rgb: &image::RgbImage, frame: &mut frame::Video, width: u32, height: u32) {
    let w = width as usize;
    let h = height as usize;
    
    // Obtener los strides primero
    let y_stride = frame.stride(0);
    let u_stride = frame.stride(1);
    let v_stride = frame.stride(2);
    
    // Convertir RGB a YUV calcula todos los valores
    let mut y_values = vec![0u8; h * y_stride];
    let mut u_values = vec![0u8; (h / 2) * u_stride];
    let mut v_values = vec![0u8; (h / 2) * v_stride];
    
    for y in 0..h {
        for x in 0..w {
            let pixel = rgb.get_pixel(x as u32, y as u32);
            let r = pixel[0] as f32;
            let g = pixel[1] as f32;
            let b = pixel[2] as f32;
            
            // Conversión RGB -> YUV (BT.601)
            let y_val = (0.257 * r + 0.504 * g + 0.098 * b + 16.0) as u8;
            y_values[y * y_stride + x] = y_val;
            
            // Submuestreo para U y V (cada 2x2 pixels)
            if y % 2 == 0 && x % 2 == 0 {
                let u_val = (-0.148 * r - 0.291 * g + 0.439 * b + 128.0) as u8;
                let v_val = (0.439 * r - 0.368 * g - 0.071 * b + 128.0) as u8;
                
                u_values[(y / 2) * u_stride + (x / 2)] = u_val;
                v_values[(y / 2) * v_stride + (x / 2)] = v_val;
            }
        }
    }
    frame.data_mut(0)[..y_values.len()].copy_from_slice(&y_values);
    frame.data_mut(1)[..u_values.len()].copy_from_slice(&u_values);
    frame.data_mut(2)[..v_values.len()].copy_from_slice(&v_values);
}