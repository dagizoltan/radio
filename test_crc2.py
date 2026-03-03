import crcmod
crc8 = crcmod.Crc(0x107, initCrc=0, rev=False, xorOut=0)
crc8.update(b"\x12\x34\x56\x78")
print("CRC8:", hex(crc8.crcValue))

crc16 = crcmod.Crc(0x18005, initCrc=0, rev=False, xorOut=0)
crc16.update(b"\x12\x34\x56\x78")
print("CRC16:", hex(crc16.crcValue))
