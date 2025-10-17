use std::path::{self, PathBuf};
use std::fs;
use clap::{command, Parser};
use ffmpeg_next::packet;
use ffmpeg_next::{
    format::{context::Output, Muxer},
    frame,
    codec,
    time,
    Rational,
    Packet, 
    Error
};
use image::{io::Reader as ImageReader, DynamicImage, RgbImage};

// Crea el video timelapse desde una carpeta con imagenes
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    #[arg(required=true)]
    carpeta: PathBuf,

    #[arg(short, long, default_value_t=10)]
    fps: u32,

    #[arg(short, long, default_value = "timelapse.mp4")]
    salida: String,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    ffmpeg_next::init().unwrap();

    let mut paths: Vec<_> = fs::read_dir(args.carpeta)?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().map_or(false, |ext| ext =="jpg" || ext == "png" || ext == "heic"))
        .collect();

    paths.sort();

    if paths.is_empty() {
        println!("No se encontraron imagenes en la carpeta");
        return Ok(());
    }

    // Configuraciones de la salida de video.
    let (width, height) = get_image_dimensions(&paths[0])?;
    let mut octx = ffmpeg_next::format::output(&args.salida)?;
    let mut stream = octx.add_stream(ffmpeg_next::codec::id::Id::H265)?;
    let mut encoder = stream.codec().encoder().video()?;

    encoder.set_width(width);
    encoder.set_height(height);
    encoder.set_format(ffmpeg_next::format::Pixel::YUV420P);
    encoder.set_time_base(Rational::new(1, args.fps as i32));

    let mut encoder = encoder.open_as(codec::Id::H265)?;
    octx.write_header()?;


    // Procesa imagenes y añade el frame
    for(i, path) in paths.iter().enumerate() {
        println!("Procesando: {}", path.display());
        let img = ImageReader::open(path)?.decode()?;
        let rgb_image = img.to_rgb8();

        let mut frame = frame::Video::new(encoder.format(), width, height);
        frame.plane_mut(0).copy_from_slice(rgb_image.as_mut());
        frame.set_pts(Some(i as i64));
        encoder.send_frame(&frame)?;

        let mut packet = Packet::empty();
        loop {
            match encoder.receive_packet(&mut packet) {
                Ok(_) => packet.write_interleaved(&mut octx)?,
                Err(Error::Again) => break,
                Err(e) => return Err((e.into()))
            }
        }
    }

    encoder.send_eof();
    let mut packet = Packet::empty();
    loop {
        match encoder.receive_packet(&mut packet) {
            Ok(_) => packet.write_interleaved(&mut octx)?,
            Err(Error::Again) => break,
            Err(e) => return Err((e.into()))
        }
    }

    octx.write_trailer()?;
    println!("\nVideo timelapse guardado como '{}' exitosamente", args.salida);
    Ok(());
}

fn get_image_dimensions(path: &PathBuf) -> Result<(u32, u32), image::ImageError> {
    let dim = ImageReader::open(path)?.into_dimensions()?;
    Ok(dim)
}