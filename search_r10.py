import asyncio
from bleak import BleakScanner

async def scan_for_device(target_mac="E4:7B:DD:3F:78:D7"):
    print(f"Scanning for devices, especially {target_mac}...")
    
    # The 'discover' method returns a list of all devices found during the scan
    devices = await BleakScanner.discover(timeout=10.0, return_adv=True)
    
    found = False
    # 'devices' is a dictionary where the key is the device's address
    for address, (device, adv_data) in devices.items():
        print(f"Found: {device.name} - {address}")
        if address.upper() == target_mac.upper():
            print(f"\n--- SUCCESS! Device {target_mac} found! ---")
            found = True
            
    if not found:
        print(f"\n--- Device {target_mac} NOT found in this scan. ---")

if __name__ == "__main__":
    asyncio.run(scan_for_device())
