/*
 * WVU Bioinstrumentation Studio reference firmware, Phase 1.
 * Teaching/engineering bench use only. This is not a medical device.
 * It sends one raw 12-bit A0-A5 channel at 1000 samples/s over USB.
 * Pulse-ox candidate pins D4, D5, D6 are safety outputs and are never HIGH.
 */
#include <Arduino.h>

static const uint8_t MAGIC[] = {'B','M','E','G'};
static const uint8_t PROTOCOL_MAJOR = 0, PROTOCOL_MINOR = 1;
static const uint16_t MAX_PAYLOAD = 1024;
static const uint32_t FIRMWARE_BUILD = 0x00010000UL;
static const uint32_t COMMAND_TIMEOUT_US = 5000000UL;
static const uint8_t LED_PINS[] = {4, 5, 6};
static const uint8_t BATCH_SAMPLES = 10;

enum MessageType : uint8_t { HELLO=1, CAPABILITIES=2, CONFIGURE=3, CONFIG_ACK=4, START=5, STOP=6, STATUS=7, SAMPLE_BATCH=8, ERROR_MESSAGE=10, PING=11, PONG=12 };
uint32_t packetSequence = 0, sampleSequence = 0, nextSampleMicros = 0, lastCommandMicros = 0;
uint64_t nextSampleTimestampUs = 0;
uint32_t microsLast = 0; uint64_t microsHigh = 0;
bool configured = false, acquiring = false; uint8_t analogPin = A0;
uint16_t batch[BATCH_SAMPLES]; uint8_t batchCount = 0; uint64_t batchFirstTimestamp = 0;

uint16_t crc16(const uint8_t* bytes, size_t length) { uint16_t crc=0xFFFF; while (length--) { crc ^= (uint16_t)(*bytes++) << 8; for (uint8_t i=0;i<8;i++) crc = (crc & 0x8000) ? (uint16_t)((crc << 1) ^ 0x1021) : (uint16_t)(crc << 1); } return crc; }
void writeU16(uint8_t* p, uint16_t v) { p[0]=v; p[1]=v>>8; } void writeU32(uint8_t* p, uint32_t v) { for(uint8_t i=0;i<4;i++) p[i]=v>>(8*i); } void writeU64(uint8_t* p, uint64_t v) { for(uint8_t i=0;i<8;i++) p[i]=v>>(8*i); }
uint64_t extendedMicros() { uint32_t now=micros(); if(now < microsLast) microsHigh += (1ULL<<32); microsLast=now; return microsHigh | now; }
void forceSafeOutputs() { for(uint8_t pin : LED_PINS) digitalWrite(pin, LOW); }
void sendFrame(uint8_t type, const uint8_t* payload, uint16_t length) {
  // All Phase 1 packets are at most a 10-sample one-channel batch (40 payload bytes).
  // Send one contiguous buffer so the USB stream cannot split a header from its CRC.
  if (length > 64 || length > MAX_PAYLOAD) return;
  // Static lifetime lets USB transmit complete a nonblocking sample write before the
  // next 10 ms batch reuses this storage. The sketch is single-threaded.
  static uint8_t packet[14 + 64 + 2];
  memcpy(packet, MAGIC, 4);
  packet[4] = PROTOCOL_MAJOR; packet[5] = PROTOCOL_MINOR; packet[6] = type; packet[7] = 0;
  writeU16(packet + 8, length); writeU32(packet + 10, packetSequence++);
  if (length > 0) memcpy(packet + 14, payload, length);
  uint16_t crc = crc16(packet, 14 + length);
  writeU16(packet + 14 + length, crc);
  Serial.write(packet, 16 + length);
  // Identity/command frames can be issued back-to-back; flush those before the static
  // buffer is reused. SAMPLE_BATCH frames are 10 ms apart and must remain nonblocking
  // to preserve the requested 1000 samples/s schedule.
  if (type != SAMPLE_BATCH) Serial.flush();
}
void sendHello() { uint8_t p[12]; writeU32(p,FIRMWARE_BUILD); writeU32(p,0x554E4F34UL); p[8]=1; p[9]=12; p[10]=1; p[11]=0; sendFrame(HELLO,p,sizeof(p)); }
void sendCapabilities() { uint8_t p[] = {12,1,6,0}; sendFrame(CAPABILITIES,p,sizeof(p)); }
void sendError(uint8_t code) { uint8_t p[] = {code}; sendFrame(ERROR_MESSAGE,p,1); }
void sendBatch() { if(batchCount==0) return; uint8_t p[20+BATCH_SAMPLES*2]; writeU32(p,sampleSequence-batchCount); writeU64(p+4,batchFirstTimestamp); writeU32(p+12,1000); p[16]=1; p[17]=batchCount; writeU16(p+18,1); for(uint8_t i=0;i<batchCount;i++) writeU16(p+20+i*2,batch[i]); sendFrame(SAMPLE_BATCH,p,20+batchCount*2); batchCount=0; }

uint8_t input[14+1024+2]; uint16_t inputLength=0;
void handleFrame(const uint8_t* frame, uint16_t length) { if(length<16 || memcmp(frame,MAGIC,4)!=0) return; uint16_t payloadLength=frame[8]|((uint16_t)frame[9]<<8); if(payloadLength>MAX_PAYLOAD || length != 14+payloadLength+2 || frame[4]!=0 || frame[5]!=1) { acquiring=false; forceSafeOutputs(); return; } uint16_t received=frame[14+payloadLength]|((uint16_t)frame[15+payloadLength]<<8); if(crc16(frame,14+payloadLength)!=received) { acquiring=false; forceSafeOutputs(); return; } const uint8_t* p=frame+14; lastCommandMicros=micros(); if(frame[6]==CONFIGURE) { if(payloadLength!=8 || p[0]!=0xE8 || p[1]!=0x03 || p[2]||p[3] || p[4]!=12 || p[5]!=1 || p[6]>5 || p[7]!=0) { acquiring=false; configured=false; forceSafeOutputs(); sendError(1); return; } analogPin=(uint8_t)(A0+p[6]); configured=true; sendFrame(CONFIG_ACK,nullptr,0); } else if(frame[6]==START) { if(!configured || payloadLength!=0) { sendError(2); return; } acquiring=true; batchCount=0; nextSampleMicros=micros(); nextSampleTimestampUs=extendedMicros(); sendFrame(STATUS,nullptr,0); } else if(frame[6]==STOP) { acquiring=false; batchCount=0; forceSafeOutputs(); sendFrame(STATUS,nullptr,0); } else if(frame[6]==PING) { /* A late-opening host can explicitly request immutable v0.1 identity frames. */ sendHello(); sendCapabilities(); sendFrame(PONG,nullptr,0); } }
void readCommands() { while(Serial.available()) { uint8_t b=(uint8_t)Serial.read(); if(inputLength<sizeof(input)) input[inputLength++]=b; else inputLength=0; while(inputLength>=4 && memcmp(input,MAGIC,4)!=0) { memmove(input,input+1,--inputLength); } if(inputLength>=14) { uint16_t n=input[8]|((uint16_t)input[9]<<8); uint16_t total=14+n+2; if(n>MAX_PAYLOAD) { memmove(input,input+1,--inputLength); } else if(inputLength>=total) { handleFrame(input,total); memmove(input,input+total,inputLength-total); inputLength-=total; } } } }
void setup() { for(uint8_t pin : LED_PINS) { pinMode(pin,OUTPUT); digitalWrite(pin,LOW); } analogReadResolution(12); Serial.begin(115200); uint32_t until=millis()+1000; while(!Serial && millis()<until){} lastCommandMicros=micros(); sendHello(); sendCapabilities(); }
void loop() { forceSafeOutputs(); readCommands(); uint32_t now=micros(); if(acquiring && (uint32_t)(now-lastCommandMicros)>COMMAND_TIMEOUT_US) { acquiring=false; batchCount=0; forceSafeOutputs(); } if(acquiring && (int32_t)(now-nextSampleMicros)>=0) { /* Timestamp the deterministic 1000 Hz schedule, not transient USB flush jitter. */ uint64_t timestamp=nextSampleTimestampUs; nextSampleTimestampUs += 1000; if(batchCount==0) batchFirstTimestamp=timestamp; batch[batchCount++]=(uint16_t)analogRead(analogPin); sampleSequence++; nextSampleMicros += 1000; if(batchCount==BATCH_SAMPLES) sendBatch(); } }
