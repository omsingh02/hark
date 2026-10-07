use std::path::Path;
use crate::models::JioSaavnSong;

pub fn tag_m4a_file(path: &Path, song: &JioSaavnSong, cover_bytes: Option<&[u8]>) -> Result<(), String> {
    let mut tag = mp4ameta::Tag::read_from_path(path)
        .map_err(|e| format!("Failed to read MP4 tags from {}: {}", path.display(), e))?;

    tag.set_title(song.clean_title());
    tag.set_artist(song.clean_artist());

    if let Some(ref alb) = song.clean_album() {
        tag.set_album(alb);
    }

    if let Some(ref yr) = song.year {
        tag.set_year(yr);
    }

    if let Some(bytes) = cover_bytes {
        // Embed high-resolution JPEG artwork
        tag.add_artwork(mp4ameta::Img::jpeg(bytes));
    }

    tag.write_to_path(path)
        .map_err(|e| format!("Failed to write MP4 tags to {}: {}", path.display(), e))?;

    Ok(())
}
