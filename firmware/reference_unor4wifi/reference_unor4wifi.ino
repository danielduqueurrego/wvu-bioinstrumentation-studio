/*
 * WVU Bioinstrumentation Studio controlled UNO R4 WiFi firmware, Phase 4.
 * Teaching use only; not a medical device. This sketch performs no physiological
 * interpretation. Analog channels are read in deterministic sequential order
 * within one logical timestamped frame; they are not literally simultaneous.
 *
 * Protocol 0.2 supports only two acquisition modes:
 *   0 simultaneous analog frame: 1..6 A0..A5 channels
 *   1 fixed pulse-ox cycle: A0 TX/A1 RX, RED/DARK/IR/DARK at 1000 us/state
 *
 * D4 green, D5 red, and D6 IR are active-HIGH. `forceSafeOutputs` runs on
 * startup, stop, malformed configuration, protocol error, watchdog timeout,
 * and every idle transition. RED and IR are never HIGH together.
 */
#include <Arduino.h>

static const uint8_t MAGIC[] = {'B','M','E','G'};
static const uint8_t PROTOCOL_MAJOR = 0, PROTOCOL_MINOR = 2;
static const uint16_t MAX_PAYLOAD = 1024;
static const uint32_t CONTROLLED_SERIAL_BAUD = 921600UL;
static const uint32_t FIRMWARE_BUILD = 0x00010002UL;
static const uint32_t DEVICE_ID = 0x554E4F34UL;
static const uint32_t COMMAND_TIMEOUT_US = 5000000UL;
static const uint8_t D4_GREEN = 4, D5_RED = 5, D6_IR = 6;
static const uint8_t BATCH_RECORDS = 10, MAX_FIELDS = 8;

enum MessageType : uint8_t { HELLO=1, CAPABILITIES=2, CONFIGURE=3, CONFIG_ACK=4, START=5, STOP=6, STATUS=7, SAMPLE_BATCH=8, ERROR_MESSAGE=10, PING=11, PONG=12 };
enum AcquisitionMode : uint8_t { SIMULTANEOUS=0, PULSEOX_4STATE=1 };

uint32_t packetSequence = 0, recordSequence = 0, nextTickMicros = 0, lastCommandMicros = 0;
uint32_t microsLast = 0; uint64_t microsHigh = 0;
uint64_t batchFirstTimestamp = 0; uint32_t batchFirstSequence = 0;
bool configured = false, acquiring = false, greenEnabled = false, overflowObserved = false;
uint8_t acquisitionMode = SIMULTANEOUS, adcBits = 12, channelCount = 1, analogPins[6] = {A0};
uint32_t frameRateHz = 1000, stateDwellUs = 1000; uint8_t pulseState = 0, batchCount = 0;
uint16_t batch[BATCH_RECORDS][MAX_FIELDS];

uint16_t crc16(const uint8_t* bytes, size_t length) { uint16_t crc=0xFFFF; while (length--) { crc ^= (uint16_t)(*bytes++) << 8; for(uint8_t i=0;i<8;i++) crc=(crc&0x8000)?(uint16_t)((crc<<1)^0x1021):(uint16_t)(crc<<1); } return crc; }
void writeU16(uint8_t* p,uint16_t v){p[0]=v;p[1]=v>>8;} void writeU32(uint8_t* p,uint32_t v){for(uint8_t i=0;i<4;i++)p[i]=v>>(8*i);} void writeU64(uint8_t* p,uint64_t v){for(uint8_t i=0;i<8;i++)p[i]=v>>(8*i);}
uint64_t extendedMicros(){uint32_t now=micros();if(now<microsLast)microsHigh+=(1ULL<<32);microsLast=now;return microsHigh|now;}

void forceSafeOutputs(){ digitalWrite(D4_GREEN,LOW); digitalWrite(D5_RED,LOW); digitalWrite(D6_IR,LOW); }
uint8_t outputMask(){ return (digitalRead(D4_GREEN)==HIGH?0x01:0) | (digitalRead(D5_RED)==HIGH?0x02:0) | (digitalRead(D6_IR)==HIGH?0x04:0); }
void applyPulseState(uint8_t state){
  digitalWrite(D4_GREEN,LOW);
  digitalWrite(D5_RED, state==0 ? HIGH : LOW);
  digitalWrite(D6_IR, state==2 ? HIGH : LOW);
}

void sendFrame(uint8_t type,const uint8_t* payload,uint16_t length){
  if(length>MAX_PAYLOAD)return;
  static uint8_t packet[14+MAX_PAYLOAD+2];
  memcpy(packet,MAGIC,4); packet[4]=PROTOCOL_MAJOR; packet[5]=PROTOCOL_MINOR; packet[6]=type; packet[7]=0;
  writeU16(packet+8,length); writeU32(packet+10,packetSequence++);
  if(length)memcpy(packet+14,payload,length);
  writeU16(packet+14+length,crc16(packet,14+length));
  Serial.write(packet,16+length);
  if(type!=SAMPLE_BATCH) Serial.flush();
}
void sendHello(){uint8_t p[12];writeU32(p,FIRMWARE_BUILD);writeU32(p+4,DEVICE_ID);p[8]=1;p[9]=14;p[10]=6;p[11]=0x03;sendFrame(HELLO,p,sizeof(p));}
void sendCapabilities(){uint8_t p[]={12,14,6,0x03,0x07,3,200,0,250,0,232,3};sendFrame(CAPABILITIES,p,sizeof(p));}
void sendError(uint8_t code){uint8_t p[]={code};sendFrame(ERROR_MESSAGE,p,1);}
// STATUS v0.2 payload: acquiring (0/1), output mask (bit 0 D4, bit 1 D5, bit 2 D6).
void sendStatus(){uint8_t p[]={acquiring?1:0,outputMask()};sendFrame(STATUS,p,sizeof(p));}

void sendBatch(){
  if(!batchCount)return;
  uint8_t fields = acquisitionMode==PULSEOX_4STATE ? 8 : channelCount;
  uint16_t length = 20 + (uint16_t)batchCount * fields * 2;
  uint8_t p[20+BATCH_RECORDS*MAX_FIELDS*2];
  writeU32(p,batchFirstSequence); writeU64(p+4,batchFirstTimestamp);
  writeU32(p+12, acquisitionMode==PULSEOX_4STATE ? stateDwellUs*4UL : 1000000UL/frameRateHz);
  p[16]=fields;p[17]=batchCount;writeU16(p+18,0x0001 | (overflowObserved?0x0004:0));
  uint16_t at=20;
  for(uint8_t record=0;record<batchCount;record++) for(uint8_t field=0;field<fields;field++){writeU16(p+at,batch[record][field]);at+=2;}
  sendFrame(SAMPLE_BATCH,p,length); batchCount=0; overflowObserved=false;
}

void rejectConfiguration(uint8_t code){acquiring=false;configured=false;batchCount=0;forceSafeOutputs();sendError(code);}
bool uniquePins(const uint8_t* pins,uint8_t count){for(uint8_t i=0;i<count;i++)for(uint8_t j=i+1;j<count;j++)if(pins[i]==pins[j])return false;return true;}
bool configureSimultaneous(const uint8_t* p,uint16_t length){
  if(length<9)return false;
  uint8_t bits=p[1], count=p[6]; uint32_t rate=(uint32_t)p[2]|((uint32_t)p[3]<<8)|((uint32_t)p[4]<<16)|((uint32_t)p[5]<<24);
  if(count<1||count>6||length!=(uint16_t)(8+count)||!(bits==12||bits==14)||rate==0||rate>1000)return false;
  uint8_t pins[6]; for(uint8_t i=0;i<count;i++){if(p[7+i]>5)return false;pins[i]=p[7+i];}
  if(!uniquePins(pins,count) || (p[7+count]&~0x01))return false;
  acquisitionMode=SIMULTANEOUS; adcBits=bits; channelCount=count; frameRateHz=rate; greenEnabled=(p[7+count]&0x01)!=0;
  for(uint8_t i=0;i<count;i++)analogPins[i]=A0+pins[i]; analogReadResolution(adcBits); return true;
}
bool configurePulseox(const uint8_t* p,uint16_t length){
  uint32_t dwell=(uint32_t)p[2]|((uint32_t)p[3]<<8)|((uint32_t)p[4]<<16)|((uint32_t)p[5]<<24);
  if(length!=10||p[1]!=14||dwell!=1000||p[6]!=2||p[7]!=0||p[8]!=1||p[9]!=0)return false;
  acquisitionMode=PULSEOX_4STATE;adcBits=14;channelCount=2;analogPins[0]=A0;analogPins[1]=A1;stateDwellUs=dwell;greenEnabled=false;analogReadResolution(14);return true;
}

void handleFrame(const uint8_t* frame,uint16_t length){
  if(length<16||memcmp(frame,MAGIC,4)!=0)return;
  uint16_t payloadLength=frame[8]|((uint16_t)frame[9]<<8);
  if(payloadLength>MAX_PAYLOAD||length!=14+payloadLength+2||frame[4]!=PROTOCOL_MAJOR||frame[5]!=PROTOCOL_MINOR){rejectConfiguration(1);return;}
  uint16_t received=frame[14+payloadLength]|((uint16_t)frame[15+payloadLength]<<8);
  if(crc16(frame,14+payloadLength)!=received){rejectConfiguration(2);return;}
  const uint8_t* p=frame+14;lastCommandMicros=micros();
  if(frame[6]==CONFIGURE){
    bool ok=payloadLength>=1 && ((p[0]==SIMULTANEOUS&&configureSimultaneous(p,payloadLength)) || (p[0]==PULSEOX_4STATE&&configurePulseox(p,payloadLength)));
    if(!ok){rejectConfiguration(3);return;} forceSafeOutputs();configured=true;sendFrame(CONFIG_ACK,nullptr,0);
  }else if(frame[6]==START){
    if(!configured||payloadLength){sendError(4);return;} forceSafeOutputs();if(greenEnabled)digitalWrite(D4_GREEN,HIGH);acquiring=true;batchCount=0;pulseState=0;recordSequence=0;nextTickMicros=micros();lastCommandMicros=micros();sendStatus();
  }else if(frame[6]==STOP){acquiring=false;batchCount=0;forceSafeOutputs();sendStatus();
  }else if(frame[6]==PING){sendHello();sendCapabilities();sendFrame(PONG,nullptr,0);}
}

uint8_t input[14+MAX_PAYLOAD+2];uint16_t inputLength=0;
void readCommands(){
  while(Serial.available()){
    uint8_t value=(uint8_t)Serial.read();
    if(inputLength<sizeof(input))input[inputLength++]=value;else{inputLength=0;rejectConfiguration(5);}
    while(inputLength>=16){
      if(memcmp(input,MAGIC,4)!=0){memmove(input,input+1,--inputLength);continue;}
      uint16_t payloadLength=input[8]|((uint16_t)input[9]<<8);uint16_t total=16+payloadLength;
      if(payloadLength>MAX_PAYLOAD){memmove(input,input+1,--inputLength);rejectConfiguration(6);continue;}
      if(inputLength<total)break;
      handleFrame(input,total);memmove(input,input+total,inputLength-total);inputLength-=total;
    }
  }
}

void acquireSimultaneous(){
  uint64_t timestamp=extendedMicros();
  if(batchCount==0){batchFirstTimestamp=timestamp;batchFirstSequence=recordSequence;}
  for(uint8_t i=0;i<channelCount;i++)batch[batchCount][i]=(uint16_t)analogRead(analogPins[i]);
  batchCount++;recordSequence++;if(batchCount>=BATCH_RECORDS)sendBatch();
}
void acquirePulseState(){
  applyPulseState(pulseState);
  uint64_t timestamp=extendedMicros();
  if(pulseState==0&&batchCount==0){batchFirstTimestamp=timestamp;batchFirstSequence=recordSequence;}
  uint16_t tx=(uint16_t)analogRead(analogPins[0]);uint16_t rx=(uint16_t)analogRead(analogPins[1]);
  batch[batchCount][pulseState]=tx;batch[batchCount][4+pulseState]=rx;pulseState++;
  if(pulseState==4){pulseState=0;batchCount++;recordSequence++;if(batchCount>=BATCH_RECORDS)sendBatch();}
}

void setup(){Serial.begin(CONTROLLED_SERIAL_BAUD);pinMode(D4_GREEN,OUTPUT);pinMode(D5_RED,OUTPUT);pinMode(D6_IR,OUTPUT);forceSafeOutputs();analogReadResolution(12);lastCommandMicros=micros();sendHello();sendCapabilities();}
void loop(){
  readCommands();
  if(!acquiring){forceSafeOutputs();return;}
  if((uint32_t)(micros()-lastCommandMicros)>COMMAND_TIMEOUT_US){acquiring=false;batchCount=0;forceSafeOutputs();sendError(7);return;}
  uint32_t now=micros();uint8_t serviced=0;
  uint32_t interval=acquisitionMode==PULSEOX_4STATE?stateDwellUs:1000000UL/frameRateHz;
  while((int32_t)(now-nextTickMicros)>=0 && serviced<3){
    if(acquisitionMode==PULSEOX_4STATE)acquirePulseState();else acquireSimultaneous();
    nextTickMicros+=interval;serviced++;
  }
  if((int32_t)(now-nextTickMicros)>=0){overflowObserved=true;nextTickMicros=now+interval;}
}
