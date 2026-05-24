use std::error::Error;
use rusqlite::{Connection, params};
use btleplug::api::{Central, Manager as _, Peripheral as _, ScanFilter, CharPropFlags};
use btleplug::platform::Manager;
use tokio::time::{sleep, Duration};
use uuid::Uuid;

// --- Datenstruktur für die empfangenen Daten ---
#[derive(Debug)]
struct R10Data {
    ball_speed: f32,
    launch_angle: f32,
    backspin: i32,
    carry: f32,
    total: f32,
}

// --- Funktion zum Parsen der empfangenen Daten ---
fn parse_data(data: &[u8]) -> Option<R10Data> {
    if data.len() > 50 {
        Some(R10Data {
            ball_speed: f32::from_le_bytes([data[12], data[13], data[14], data[15]]),
            launch_angle: f32::from_le_bytes([data[16], data[17], data[18], data[19]]),
            backspin: f32::from_le_bytes([data[24], data[25], data[26], data[27]]) as i32,
            carry: f32::from_le_bytes([data[44], data[45], data[46], data[47]]),
            total: f32::from_le_bytes([data[48], data[49], data[50], data[51]]),
        })
    } else {
        None
    }
}

// --- Funktion zum Speichern der Daten in der SQLite-Datenbank ---
fn save_to_db(data: &R10Data, club: &str) -> Result<(), Box<dyn Error>> {
    let conn = Connection::open("golf_data.db")?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS shots (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp DATETIME,
            ball_speed REAL,
            launch_angle REAL,
            backspin INTEGER,
            carry REAL,
            total REAL,
            club TEXT
        )",
        [],
    )?;

    conn.execute(
        "INSERT INTO shots (timestamp, ball_speed, launch_angle, backspin, carry, total, club)
         VALUES (datetime('now'), ?, ?, ?, ?, ?, ?)",
        params![
            data.ball_speed,
            data.launch_angle,
            data.backspin,
            data.carry,
            data.total,
            club
        ],
    )?;
    Ok(())
}

// --- Asynchrone Hauptfunktion für die Bluetooth-Kommunikation ---
#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Bluetooth-Manager initialisieren
    let manager = Manager::new().await?;
    let adapter = manager
        .adapters()
        .await?
        .into_iter()
        .next()
        .ok_or("Kein Bluetooth-Adapter gefunden")?;

    // Scannen nach Geräten
    adapter.start_scan(ScanFilter::default()).await?;
    println!("Scanne nach Garmin R10...");
    sleep(Duration::from_secs(5)).await;
    adapter.stop_scan().await?;

    // Suche nach dem Garmin R10
    for peripheral in adapter.peripherals().await? {
        let properties = peripheral.properties().await?;
        if let Some(name) = properties.local_name {
            if name.contains("R10") {
                println!("Garmin R10 gefunden: {:?}", name);

                // Verbindung herstellen
                peripheral.connect().await?;
                println!("Verbunden mit Garmin R10");

                // Charakteristiken finden
                let characteristics = peripheral.discover_characteristics().await?;
                for characteristic in characteristics {
                    // UUIDs für den Garmin R10 (Beispielwerte, bitte anpassen!)
                    let service_uuid = Uuid::parse_str("6A4E2800-667B-11E3-949A-0800200C9A66")?;
                    let notify_uuid = Uuid::parse_str("6A4E2812-667B-11E3-949A-0800200C9A66")?;
                    let write_uuid = Uuid::parse_str("6A4E2822-667B-11E3-949A-0800200C9A66")?;
                    
                    if characteristic.uuid == notify_uuid {
                        println!("Benachrichtigungs-Charakteristik gefunden: {:?}", characteristic.uuid);

                        // Benachrichtigungen aktivieren
                        peripheral.subscribe(&characteristic).await?;

                        // Daten lesen und verarbeiten
                        let data = peripheral.read(&characteristic).await?;
                        if let Some(parsed_data) = parse_data(&data) {
                            println!(
                                "Empfangene Daten: Ballgeschwindigkeit: {} m/s, Abflugwinkel: {}°, Backspin: {} rpm, Carry: {} m, Gesamt: {} m",
                                parsed_data.ball_speed,
                                parsed_data.launch_angle,
                                parsed_data.backspin,
                                parsed_data.carry,
                                parsed_data.total
                            );
                            save_to_db(&parsed_data, "Driver")?;
                        }
                    }

                    // Optional: Schreib-Charakteristik für Handshake
                    if characteristic.uuid == write_uuid {
                        println!("Schreib-Charakteristik gefunden: {:?}", characteristic.uuid);
                        peripheral.write(&characteristic, &[0x01]).await?;
                    }
                }
            }
        }
    }

    Ok(())
}
