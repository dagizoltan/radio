import crcmod
crc8 = crcmod.Crc(0x107, initCrc=0, rev=False, xorOut=0)
crc8.update(b"123456789")
print("CRC8:", hex(crc8.crcValue))

crc16 = crcmod.Crc(0x18005, initCrc=0, rev=False, xorOut=0)
crc16.update(b"123456789")
print("CRC16:", hex(crc16.crcValue))
