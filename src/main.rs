use chbs::config::{self, BasicConfig};
use chbs::probability::Probability;
use chbs::scheme::ToScheme;
use chbs::word::{WordList, WordSampler};
use clap::{Parser, ValueEnum};
use encoding_rs::{UTF_8, WINDOWS_1252};
use encoding_rs_io::DecodeReaderBytesBuilder;
use glob::{glob, GlobError};
use serde::{Deserialize, Serialize};
use std::error::Error;
use std::fs::File;
use std::path::PathBuf;

#[macro_use]
extern crate log;

// Idee:
// Für alle möglichen csv-Formate gibt es passende structs.
// Per CLI-Argumente teilt man dem Programm mit, welche es verarbeiten soll.
// Es können Passwörter generiert werden.
// Am Ende wird eine csv-Datei erstellt, die für IServ gedacht ist.

pub const WORDLIST: &str = include_str!("../res/words.txt");

#[derive(Debug, Deserialize)]
enum Record {
    RecordSchild(RecordSchild),
    RecordGastschueler(RecordGastschueler),
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum RecordType {
    Schild,
    Gastschueler,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum Encoding {
    Utf8,
    Windows,
}

#[derive(Debug, Parser)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(default_value = ".", short, long, value_parser)]
    dirpath: String,
    #[clap(default_value = "./export", short, long, value_parser)]
    outputpath: String,
    #[clap(short, long, arg_enum, value_parser)]
    record_type: RecordType,
    #[clap(default_value_t = Encoding::Windows, short, arg_enum, long, value_parser)]
    encoding: Encoding,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RecordSchild {
    nachname: String,
    vorname: String,
    gruppe: String,
    guid: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
struct RecordGastschueler {
    #[serde(rename = "NAME, VORNAME")]
    name: String,
    stammschule: String,
    note: String,
    klasse: String,
    #[serde(rename = "SCHÜLERNR")]
    schuelernr: String,
    #[serde(rename = "FACHKÜRZEL")]
    fachkuerzel: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "PascalCase")]
struct RecordIserv {
    nachname: String,
    vorname: String,
    klasse: String,
    #[serde(rename = "Import-ID")]
    import_id: String,
    password: String,
}

impl RecordIserv {
    fn new(nachname: String, vorname: String, klasse: String, import_id: String) -> Self {
        let mut config = BasicConfig::default();
        config.words = 3;
        config.word_provider = WordList::new(
            WORDLIST
                .lines()
                .map(|w| w.to_owned())
                .collect::<Vec<String>>(),
        )
        .sampler();
        config.separator = "-".into();
        config.capitalize_first = Probability::Never;
        let scheme = config.to_scheme();
        let password = scheme.generate();
        RecordIserv {
            nachname,
            vorname,
            klasse,
            import_id,
            password,
        }
    }
}

impl From<RecordSchild> for RecordIserv {
    fn from(record: RecordSchild) -> Self {
        let klasse: String;
        // TODO Ab Schuljahr 2023/24 muss 10 entfernt und 13 hinzugefügt werden.
        if record.gruppe.starts_with("10") {
            klasse = "10".to_string();
        } else if record.gruppe.starts_with("11") {
            klasse = "11".to_string();
        } else if record.gruppe.starts_with("12") {
            klasse = "12".to_string();
        } else {
            klasse = record.gruppe
        };
        RecordIserv::new(record.nachname, record.vorname, klasse, record.guid)
    }
}

impl From<RecordGastschueler> for RecordIserv {
    fn from(record: RecordGastschueler) -> Self {
        let name: Vec<&str> = record.name.split(", ").collect();
        let nachname = String::from(&name[0][1..]);
        let vorname = name[1].split(" (G)").collect();
        RecordIserv::new(nachname, vorname, record.klasse, record.schuelernr)
    }
}

impl From<Record> for RecordIserv {
    fn from(record: Record) -> Self {
        match record {
            Record::RecordSchild(record) => record.into(),
            Record::RecordGastschueler(record) => record.into(),
        }
    }
}

fn main() {
    env_logger::init();
    let args = Args::parse();
    let paths = get_all_csv_paths(&args.dirpath).unwrap();
    let records = get_all_csv_records_in_dir(paths, args.record_type, args.encoding);
    match records {
        Ok(r) => {
            let records_iserv = &r.into_iter().map(|r| r.into()).collect();
            match write_records_to_file(records_iserv) {
                Ok(_) => (),
                Err(e) => println!("{:?}", e),
            };
        }
        Err(e) => println!("{:?}", e),
    }
}

fn get_all_csv_paths(dirpath: &str) -> Result<Vec<PathBuf>, Box<dyn Error>> {
    let mut paths: Vec<PathBuf> = Vec::new();
    let pattern = format!("{}{}", dirpath, "/*.CSV");
    let paths_glob = glob(&pattern).unwrap();
    for path in paths_glob {
        match path {
            Ok(p) => paths.push(p),
            Err(e) => println!("{:?}", e),
        }
    }
    Ok(paths)
}

fn get_all_csv_records_in_dir(
    paths: Vec<PathBuf>,
    record_type: RecordType,
    encoding: Encoding,
) -> Result<Vec<Record>, GlobError> {
    let mut records: Vec<Record> = Vec::new();
    for path in paths {
        let path_unwrapped = path;
        let file = File::open(path_unwrapped).unwrap();
        let records_file = get_all_csv_records_in_file(file, record_type, encoding);
        match records_file {
            Ok(rs) => {
                for record in rs {
                    records.push(record);
                }
            }
            Err(e) => println!("{:?}", e),
        }
    }
    debug!("Test");
    Ok(records)
}

fn get_all_csv_records_in_file(
    file: File,
    record_type: RecordType,
    encoding: Encoding,
) -> Result<Vec<Record>, Box<dyn Error>> {
    let mut records: Vec<Record> = Vec::new();
    let win_reader = match encoding {
        Encoding::Utf8 => DecodeReaderBytesBuilder::new()
            .encoding(Some(UTF_8))
            .build(file),
        Encoding::Windows => DecodeReaderBytesBuilder::new()
            .encoding(Some(WINDOWS_1252))
            .build(file),
    };

    let mut rdr = csv::ReaderBuilder::new()
        .delimiter(b';')
        .from_reader(win_reader);
    match record_type {
        RecordType::Schild => {
            for result in rdr.deserialize() {
                let record: RecordSchild = result?;
                records.push(Record::RecordSchild(record));
            }
        }
        RecordType::Gastschueler => {
            for result in rdr.deserialize() {
                let record: RecordGastschueler = result?;
                records.push(Record::RecordGastschueler(record));
            }
        }
    };

    Ok(records)
}

fn print_records_from_file(records: &Vec<Record>) {
    for record in records {
        println!("{:?}", record);
    }
}

fn write_records_to_file(records: &Vec<RecordIserv>) -> Result<(), Box<dyn Error>> {
    let mut wtr = csv::WriterBuilder::new()
        .delimiter(b';')
        .from_path("./export.csv")?;
    for record in records {
        wtr.serialize(record)?;
    }
    wtr.flush()?;
    Ok(())
}
