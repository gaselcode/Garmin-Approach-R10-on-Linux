use std::error::Error;
use rusqlite::{Connection, params};
use btleplug::api::{Central, Manager as _, Peripheral as PeripheralApi, ScanFilter, WriteType};
use btleplug::platform::{Manager, Peripheral};
use tokio::time::{sleep, Duration};
use uuid::Uuid;
use futures::stream::StreamExt;

#[derive(Debug)]
struct R10Data {
    ball_speed: f32,
    launch_angle: f32,
    backspin: i32,
    carry: f32,
    total: f32,
}

fn parse_data(data: &[u8]) -> Option<R10Data> {
    if data.len() < 52 {
        eprintln!("⚠️ Paket zu kurz: {} Bytes", data.len());
        return None;
    }
    println!("🔍 Raw HEX: {:02X?}", &data[..32.min(data.len())]);
    Some(R10Data {
        ball_speed: f32::from_le_bytes([data[12], data[13], data[14], data[15]]),
        launch_angle: f32::from_le_bytes([data[16], data[17], data[18], data[19]]),
        backspin: i32::from_le_bytes([data[24], data[25], data[26], data[27]]),
        carry: f32::from_le_bytes([data[44], data[45], data[46], data[47]]),
        total: f32::from_le_bytes([data[48], data[49], data[50], data[51]]),
    })
}

fn save_to_db(data: &R10Data, club: &str) -> Result<(), Box<dyn Error>> {
    let conn = Connection::open("golf_data.db")?;
    conn.execute(
        "CREATE TABLE IF NOT EXISTS shots (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
            ball_speed REAL, launch_angle REAL,
            backspin INTEGER, carry REAL, total REAL, club TEXT
        )",
        [],
    )?;
    conn.execute(
        "INSERT INTO shots (ball_speed, launch_angle, backspin, carry, total, club)
         VALUES (?, ?, ?, ?, ?, ?)",
        params![data.ball_speed, data.launch_angle, data.backspin, data.carry, data.total, club],
    )?;
    Ok(())
}

async fn find_r10(adapter: &btleplug::platform::Adapter) -> Result<Peripheral, Box<dyn Error>> {
    for attempt in 1..=3 {
        println!("🔎 Scan-Versuch {}/3 (10 Sekunden)...", attempt);
        adapter.start_scan(ScanFilter::default()).await?;
        sleep(Duration::from_secs(10)).await;
        adapter.stop_scan().await?;
        sleep(Duration::from_millis(500)).await;

        let peripherals = adapter.peripherals().await?;
        for p in &peripherals {
            if let Some(props) = p.properties().await? {
                if let Some(name) = props.local_name {
                    if name.contains("R10") || name.contains("Approach") || name.contains("Garmin") {
                        println!("📋 Gefunden: {} ({})", name, props.address);
                        return Ok(p.clone());
                    }
                }
            }
        }
        println!("⏳ R10 nicht gefunden. Wiederhole Scan...");
    }
    Err("R10 nach 3 Versuchen nicht erreichbar.".into())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let manager = Manager::new().await?;
    let adapter = manager.adapters().await?.into_iter().next()
        .ok_or("Kein Bluetooth-Adapter gefunden")?;

    let peripheral = find_r10(&adapter).await?;
    println!("✅ R10 ausgewählt");

    println!("🔗 Verbinde...");
    peripheral.connect().await?;
    sleep(Duration::from_secs(2)).await;

    println!("📡 Discovery GATT-Services...");
    let mut discovery_ok = false;
    
    // Retry-Loop mit automatisch Reconnect bei Timeout
    for attempt in 1..=3 {
        match peripheral.discover_services().await {
            Ok(_) => {
                discovery_ok = true;
                println!("✅ Services entdeckt");
                break;
            }
            Err(e) => {
                eprintln!("⚠️ Discovery-Versuch {} fehlgeschlagen: {}", attempt, e);
                if attempt < 3 {
                    eprintln!("🔄 Trenne und verbinde neu...");
                    let _ = peripheral.disconnect().await;
                    sleep(Duration::from_secs(1)).await;
                    let _ = peripheral.connect().await;
                    sleep(Duration::from_secs(2)).await;
                }
            }
        }
    }

    if !discovery_ok {
        return Err("GATT-Discovery dauerhaft fehlgeschlagen. Bitte prüfe Pairing-Status.".into());
    }

    let chars = peripheral.characteristics();
    println!("📋 Charakteristiken: {}", chars.len());

    let write_uuid = Uuid::parse_str("6a4e2822-667b-11e3-949a-0800200c9a66")?;
    let write_char = chars.iter().find(|c| c.uuid == write_uuid)
        .ok_or("Write-Char nicht gefunden!")?;

    println!("✉️ Sende Init-Command...");
    let _ = peripheral.write(write_char, &[0x01, 0x00], WriteType::WithoutResponse).await;
    sleep(Duration::from_secs(1)).await;

    let notify_candidates = [
        "6a4e2810-667b-11e3-949a-0800200c9a66",
        "6a4e2811-667b-11e3-949a-0800200c9a66",
        "6a4e2812-667b-11e3-949a-0800200c9a66",
    ];

    let mut subscribed_char = None;
    for uuid_str in &notify_candidates {
        let target_uuid = Uuid::parse_str(uuid_str)?;
        if let Some(nc) = chars.iter().find(|c| c.uuid == target_uuid) {
            println!("🔔 Versuche Subscription auf {}...", uuid_str);
            match peripheral.subscribe(nc).await {
                Ok(_) => {
                    subscribed_char = Some(nc);
                    println!("✅ Subscription erfolgreich!");
                    break;
                }
                Err(e) => eprintln!("⚠️ Fehler bei {}: {}", uuid_str, e),
            }
            sleep(Duration::from_millis(500)).await;
        }
    }

    let notify_char = subscribed_char.ok_or("Keine Notify-Char akzeptiert Subscription.")?;

    let mut stream = peripheral.notifications().await?;
    let mut count = 0;
    println!("🎯 Warte auf Schläge (Strg+C zum Beenden)...");

    loop {
        tokio::select! {
            Some(note) = stream.next() => {
                count += 1;
                println!("\n📦 Paket #{} ({} Bytes) von {}", count, note.value.len(), note.uuid);
                if let Some(d) = parse_data(&note.value) {
                    println!("⛳ Speed: {:.1} m/s | Angle: {:.1}° | Spin: {} rpm | Carry: {:.1} m",
                        d.ball_speed, d.launch_angle, d.backspin, d.carry);
                    let _ = save_to_db(&d, "Driver");
                }
            },
            _ = tokio::signal::ctrl_c() => {
                println!("\n🛑 Beende...");
                break;
            }
        }
    }

    let _ = peripheral.unsubscribe(notify_char).await;
    let _ = peripheral.disconnect().await;
    println!("🔌 Getrennt. Daten in: golf_data.db");
    Ok(())
}
