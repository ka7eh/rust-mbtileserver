use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::read_dir;
use std::path::PathBuf;

use rusqlite::{params, Connection, OpenFlags, NO_PARAMS};

use serde;
use serde::{Deserialize, Serialize};
use serde_json::Value as JSONValue;

use crate::errors::{Error, Result};

use crate::utils::{decode, get_data_format, DataFormat};

#[derive(Clone, Debug)]
pub struct TileMeta {
    pub path: PathBuf,
    pub name: Option<String>,
    pub version: Option<String>,
    pub tilejson: String,
    pub scheme: String,
    pub id: String,
    pub tile_format: DataFormat,
    pub grid_format: Option<DataFormat>,
    pub bounds: Option<Vec<f64>>,
    pub minzoom: Option<u32>,
    pub maxzoom: Option<u32>,
    pub description: Option<String>,
    pub attribution: Option<String>,
    pub legend: Option<String>,
    pub template: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TileSummaryJSON {
    pub image_type: DataFormat,
    pub url: String,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TileMetaJSON {
    pub name: Option<String>,
    pub version: Option<String>,
    pub map: String,
    pub tiles: Vec<String>,
    pub tilejson: String,
    pub scheme: String,
    pub id: String,
    pub format: DataFormat,
    pub grids: Option<Vec<String>>,
    pub bounds: Option<Vec<f64>>,
    pub minzoom: Option<u32>,
    pub maxzoom: Option<u32>,
    pub description: Option<String>,
    pub attribution: Option<String>,
    pub legend: Option<String>,
    pub template: Option<String>,
}

#[derive(Deserialize)]
struct UTFGridKeys {
    pub grid: Vec<String>,
    pub keys: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UTFGrid {
    pub data: HashMap<String, JSONValue>,
    pub grid: Vec<String>,
    pub keys: Vec<String>,
}

pub fn get_template(tile_path: &PathBuf) -> Result<&'static str> {
    let connection =
        Connection::open_with_flags(tile_path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    match get_data_format_via_query(&connection, "tile") {
        Ok(tile_format) => match tile_format {
            DataFormat::PBF => Ok("templates/map_vector.html"),
            _ => Ok("templates/map.html"),
        },
        _ => Err(Error),
    }
}

pub fn get_data_format_via_query(connection: &Connection, category: &str) -> Result<DataFormat> {
    let query = match category {
        "tile" => r#"SELECT tile_data FROM tiles LIMIT 1"#,
        "grid" => r#"SELECT grid_utfgrid FROM grid_utfgrid LIMIT 1"#,
        _ => return Err(Error),
    };
    let mut statement = match connection.prepare(query) {
        Ok(s) => s,
        Err(_) => return Err(Error),
    };
    let data_format: DataFormat = statement
        .query_row(NO_PARAMS, |row| {
            Ok(get_data_format(&row.get::<_, Vec<u8>>(0).unwrap()))
        })
        .unwrap_or(DataFormat::UNKNOWN);
    Ok(data_format)
}

pub fn get_tile_details<'a>(path: &PathBuf, tile_name: &str) -> Result<TileMeta> {
    let connection = match Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY) {
        Ok(connection) => connection,
        Err(_) => return Err(Error),
    };

    // 'tiles', 'metadata' tables or views must be present
    let query = r#"SELECT count(*) FROM sqlite_master WHERE name IN ('tiles', 'metadata')"#;
    let mut statement = match connection.prepare(query) {
        Ok(s) => s,
        Err(_) => return Err(Error),
    };
    match statement.query_row(NO_PARAMS, |row| Ok(row.get::<_, i8>(0).unwrap_or(0))) {
        Ok(count) => {
            if count < 2 {
                return Err(Error);
            }
        }
        Err(_) => return Err(Error),
    };

    let tile_format = match get_data_format_via_query(&connection, "tile") {
        Ok(tile_format) => tile_format,
        _ => return Err(Error),
    };

    let mut metadata = TileMeta {
        path: PathBuf::from(path),
        name: None,
        version: None,
        tilejson: String::from("2.1.0"),
        scheme: String::from("xyz"),
        id: String::from(tile_name),
        tile_format,
        grid_format: get_grid_info(&connection),
        bounds: None,
        minzoom: None,
        maxzoom: None,
        description: None,
        attribution: None,
        legend: None,
        template: None,
    };

    let mut statement = connection
        .prepare(r#"SELECT name, value FROM metadata WHERE value IS NOT ''"#)
        .unwrap();
    let mut metadata_rows = statement.query(NO_PARAMS).unwrap();

    while let Some(row) = metadata_rows.next().unwrap() {
        let label: String = row.get(0).unwrap();
        let value: String = row.get(1).unwrap();
        match label.as_ref() {
            "name" => metadata.name = Some(value),
            "version" => metadata.version = Some(value),
            "bounds" => {
                metadata.bounds = Some(value.split(",").filter_map(|s| s.parse().ok()).collect())
            }
            "minzoom" => metadata.minzoom = Some(value.parse().unwrap()),
            "maxzoom" => metadata.maxzoom = Some(value.parse().unwrap()),
            "description" => metadata.description = Some(value),
            "attribution" => metadata.attribution = Some(value),
            "legend" => metadata.legend = Some(value),
            "template" => metadata.template = Some(value),
            _ => (),
        }
    }

    Ok(metadata)
}

pub fn discover_tilesets(parent_dir: String, path: PathBuf) -> HashMap<String, TileMeta> {
    // Walk through the given path and its subfolders, find all valid mbtiles and create and return a map of mbtiles file names to their absolute path
    let mut tiles = HashMap::new();
    for p in read_dir(path).unwrap() {
        let p = p.unwrap().path();
        if p.is_dir() {
            let dir_name = p.file_stem().unwrap().to_str().unwrap();
            let mut parent_dir_cloned = parent_dir.clone();
            parent_dir_cloned.push_str(dir_name);
            parent_dir_cloned.push_str("/");
            tiles.extend(discover_tilesets(parent_dir_cloned, p));
        } else if p.extension().and_then(OsStr::to_str) == Some("mbtiles") {
            let file_name = p.file_stem().and_then(OsStr::to_str).unwrap();
            let mut parent_dir_cloned = parent_dir.clone();
            parent_dir_cloned.push_str(file_name);
            match get_tile_details(&p, file_name) {
                Ok(tile_meta) => tiles.insert(parent_dir_cloned, tile_meta),
                Err(_) => None,
            };
        }
    }
    tiles
}

fn get_grid_info(connection: &Connection) -> Option<DataFormat> {
    let mut statement = connection.prepare(r#"SELECT count(*) FROM sqlite_master WHERE name IN ('grids', 'grid_data', 'grid_utfgrid', 'keymap', 'grid_key')"#).unwrap();
    let count: u8 = statement
        .query_row(NO_PARAMS, |row| Ok(row.get(0).unwrap()))
        .unwrap();
    if count == 5 {
        match get_data_format_via_query(connection, "grid") {
            Ok(grid_format) => return Some(grid_format),
            Err(_) => return None,
        };
    }
    None
}

pub fn get_grid_data(tile_path: &PathBuf, z: u32, x: u32, y: u32) -> Result<UTFGrid> {
    let connection =
        Connection::open_with_flags(tile_path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let mut statement = connection
        .prepare(
            r#"SELECT grid
                 FROM grids
                WHERE zoom_level = ?1
                  AND tile_column = ?2
                  AND tile_row = ?3
            "#,
        )
        .unwrap();
    let grid_data = match statement.query_row(params![z, x, y], |row| {
        Ok(row.get::<_, Vec<u8>>(0).unwrap())
    }) {
        Ok(d) => d,
        Err(_) => return Err(Error),
    };
    let data_format = get_data_format(&grid_data);
    let grid_key_json: UTFGridKeys =
        serde_json::from_str(&decode(grid_data, data_format).unwrap()).unwrap();
    let mut grid_data = UTFGrid {
        data: HashMap::new(),
        grid: grid_key_json.grid,
        keys: grid_key_json.keys,
    };

    let mut statement = connection
        .prepare(
            r#"SELECT key_name, key_json
                 FROM grid_data
                WHERE zoom_level = ?1
                  AND tile_column = ?2
                  AND tile_row = ?3
            "#,
        )
        .unwrap(); // TODO handle error
    let grid_data_iter = statement
        .query_map(params![z, x, y], |row| {
            Ok((
                row.get::<_, String>(0).unwrap(),
                row.get::<_, String>(1).unwrap(),
            ))
        })
        .unwrap();
    for gd in grid_data_iter {
        let (k, v) = gd.unwrap();
        let v: JSONValue = serde_json::from_str(&v).unwrap();
        grid_data.data.insert(k, v);
    }

    Ok(grid_data)
}

pub fn get_tile_data(tile_path: &PathBuf, z: u32, x: u32, y: u32) -> Vec<u8> {
    let connection =
        Connection::open_with_flags(tile_path, OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();

    let mut statement = connection
        .prepare(
            r#"SELECT tile_data
                FROM tiles
                WHERE zoom_level = ?1
                  AND tile_column = ?2
                  AND tile_row = ?3
            "#,
        )
        .unwrap(); // TODO handle error
    statement
        .query_row(params![z, x, y], |row| Ok(row.get(0).unwrap()))
        .unwrap_or(get_blank_image())
}

pub fn get_blank_image() -> Vec<u8> {
    let image = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x01\x00\x00\x00\x01\x00\x01\x03\x00\x00\x00f\xbc:%\x00\x00\x00\x03PLTE\x00\x00\x00\xa7z=\xda\x00\x00\x00\x01tRNS\x00@\xe6\xd8f\x00\x00\x00\x1fIDATh\xde\xed\xc1\x01\r\x00\x00\x00\xc2 \xfb\xa76\xc77`\x00\x00\x00\x00\x00\x00\x00\x00q\x07!\x00\x00\x01\xa7W)\xd7\x00\x00\x00\x00IEND\xaeB`\x82";
    image.to_vec()
}
