use calamine::{open_workbook, Error, RangeDeserializerBuilder, Reader, Xlsx};
use chbs::config::BasicConfig;
use chbs::probability::Probability;
use chbs::scheme::ToScheme;
use chbs::word::WordList;
use clap::{Parser, ValueEnum};
use encoding_rs::{UTF_8, WINDOWS_1252};
use encoding_rs_io::DecodeReaderBytesBuilder;
use serde::{Deserialize, Serialize};
use std::error::Error as OtherError;
use std::fs::File;
use std::path::PathBuf;

use log::{error, info};

// Idee:
// Für alle möglichen csv-Formate gibt es passende structs.
// Per CLI-Argumente teilt man dem Programm mit, welche es verarbeiten soll.
// Es werden Passwörter generiert werden.
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
enum FileType {
    Csv,
    Excel,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, ValueEnum)]
enum Encoding {
    Utf8,
    Windows,
}

#[derive(Debug, Parser)]
#[clap(author, version, about, long_about = None)]
struct Args {
    #[clap(short, long, value_parser)]
    file_path: String,
    #[clap(default_value = "./import_iserv_ready.csv", short, long, value_parser)]
    output_path: String,
    #[clap(default_value_t = RecordType::Schild ,short, long, arg_enum, value_parser)]
    record_type: RecordType,
    #[clap(default_value_t = FileType::Csv, short = 't', long, arg_enum, value_parser)]
    file_type: FileType,
    #[clap(default_value_t = Encoding::Utf8, short, arg_enum, long, value_parser)]
    encoding: Encoding,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RecordSchild {
    nachname: String,
    vorname: String,
    klasse: String,
    #[serde(rename = "eindeutige Nummer (GUID)")]
    guid: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
struct RecordGastschueler {
    #[serde(rename = "NAME, VORNAME")]
    name: String,
    klasse: String,
    #[serde(rename = "SCHÜLERNR")]
    schuelernr: String,
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
        config.words = 2;
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
        if record.klasse.starts_with("11") {
            klasse = "11".to_string();
        } else if record.klasse.starts_with("12") {
            klasse = "12".to_string();
        } else if record.klasse.starts_with("13") {
            klasse = "13".to_string();
        } else {
            klasse = record.klasse
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
    info!("Programm gestartet.");
    let args = Args::parse();
    let records: Result<Vec<Record>, _>;
    let path = PathBuf::from(args.file_path);
    info!("Öffne nun Datei.");
    match args.file_type {
        FileType::Csv => {
            records = get_all_csv_records_in_file(path, args.record_type, args.encoding);
        }
        FileType::Excel => {
            records = get_all_xlsx_records_in_file(path, args.record_type);
        }
    }
    info!("Schreibe in Datei.");
    match records {
        Ok(r) => {
            let records_iserv = &r.into_iter().map(|r| r.into()).collect();
            match write_records_to_file(records_iserv, args.output_path) {
                Ok(_) => (),
                Err(e) => println!("{:?}", e),
            };
        }
        Err(e) => println!("{:?}", e),
    }
    info!("Beende das Programm.");
}

fn get_all_xlsx_records_in_file(
    path: PathBuf,
    record_type: RecordType,
) -> Result<Vec<Record>, Box<dyn OtherError>> {
    let mut records: Vec<Record> = Vec::new();
    let mut workbook: Xlsx<_> = open_workbook(path)?;
    info!("Excel-Datei geöffnet.");
    let sheets = workbook.sheet_names().to_owned();
    let range = workbook
        .worksheet_range(&sheets[0])
        .ok_or(Error::Msg("Cannot find 'Sheet1'"))??;
    match record_type {
        RecordType::Schild => {
            let iter = RangeDeserializerBuilder::new().from_range(&range)?;
            for row in iter {
                let record = row?;
                records.push(Record::RecordSchild(record));
            }
        }
        RecordType::Gastschueler => {
            let iter = RangeDeserializerBuilder::new().from_range(&range)?;
            for row in iter {
                let record = row?;
                records.push(Record::RecordGastschueler(record));
            }
        }
    }

    Ok(records)
}

fn get_all_csv_records_in_file(
    path: PathBuf,
    record_type: RecordType,
    encoding: Encoding,
) -> Result<Vec<Record>, Box<dyn OtherError>> {
    let file = File::open(path).unwrap();
    info!("CSV-Datei geöffnet.");
    let mut records: Vec<Record> = Vec::new();
    info!("Checke Encoding.");
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

fn write_records_to_file(
    records: &Vec<RecordIserv>,
    path: String,
) -> Result<(), Box<dyn OtherError>> {
    let mut wtr = csv::WriterBuilder::new().delimiter(b';').from_path(path)?;
    for record in records {
        wtr.serialize(record)?;
    }
    wtr.flush()?;
    Ok(())
}
