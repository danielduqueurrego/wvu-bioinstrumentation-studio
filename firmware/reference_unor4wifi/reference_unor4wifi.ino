/*
 * WVU Bioinstrumentation Studio controlled UNO R4 WiFi firmware.
 * Teaching use only; not a medical device. This sketch performs no physiological
 * interpretation. Analog channels are read in deterministic sequential order
 * within one logical timestamped record; they are not literally simultaneous.
 *
 * The desktop's CONFIGURE command sets the analog pins, the timing, and the
 * D4/D5/D6 outputs of every recording; no lab's pins or timing are fixed here.
 * Protocol 0.4 supports three acquisition modes:
 *   0 simultaneous analog: 1-6 distinct pins of A0-A5, one record per frame
 *     at 100, 200, 250, 500, or 1000 frames/s
 *   1 legacy pulse-ox 4-state: one TX and one RX pin, RED/DARK 1/IR/DARK 2 at
 *     250-5000 us per state, one 8-field record per cycle
 *   2 LED-sequenced pulse-ox: 1-6 distinct pins, the same RED/DARK 1/IR/DARK 2
 *     sequence at 1000-5000 us per state, one record per state: the state
 *     code, then every pin, read while that state's LED is still on
 *
 * D4 green, D5 red, and D6 IR are active-HIGH. `forceSafeOutputs` runs on
 * startup, stop, every configuration (accepted or malformed), protocol error,
 * host-command timeout, and every idle transition. RED and IR are never HIGH
 * together. A hardware watchdog resets a stalled board; its pins are then
 * inputs until setup() drives them LOW again.
 */
#include <Arduino.h>
#include <WDT.h>

static const uint8_t MAGIC[] = {'B','M','E','G'};
static const uint8_t PROTOCOL_MAJOR = 0, PROTOCOL_MINOR = 4;
static const uint16_t MAX_PAYLOAD = 1024;
static const uint32_t CONTROLLED_SERIAL_BAUD = 921600UL;
static const uint32_t FIRMWARE_BUILD = 0x00010007UL;
static const uint32_t DEVICE_ID = 0x554E4F34UL;
static const uint32_t COMMAND_TIMEOUT_US = 5000000UL;
static const uint32_t HARDWARE_WATCHDOG_MS = 1000;
static const uint8_t D4_GREEN = 4, D5_RED = 5, D6_IR = 6;
static const uint8_t BATCH_RECORDS = 10, MAX_FIELDS = 8;
static const uint8_t ACQUISITION_BATCH_SLOTS = 4;
static const uint8_t TX_QUEUE_CAPACITY = 8, TX_CONTROL_RESERVE = 3;
static const uint8_t COMMAND_BYTES_PER_PASS = 64;
// Batch encoding waits while the next sample is due within this window.
static const uint32_t ENCODE_WINDOW_US = 120;
static const uint16_t SAMPLE_FLAG_ACQUIRING = 0x0001;
static const uint16_t SAMPLE_FLAG_QUEUE_OVERFLOW = 0x0004;
static const uint16_t SAMPLE_FLAG_EXPLICIT_TIMESTAMPS = 0x0008;
static const uint16_t SAMPLE_FLAG_DEADLINE_SKIPPED = 0x0010;
static const uint16_t MAX_SAMPLE_PAYLOAD = 24 + BATCH_RECORDS * (8 + MAX_FIELDS * 2);
static const uint16_t MAX_TX_FRAME = 14 + MAX_SAMPLE_PAYLOAD + 2;

enum MessageType : uint8_t { HELLO=1, CAPABILITIES=2, CONFIGURE=3, CONFIG_ACK=4, START=5, STOP=6, STATUS=7, SAMPLE_BATCH=8, ERROR_MESSAGE=10, PING=11, PONG=12 };
enum AcquisitionMode : uint8_t { SIMULTANEOUS=0, PULSEOX_4STATE=1, PULSEOX_SEQUENCED=2 };

uint32_t packetSequence = 0, recordSequence = 0, nextTickMicros = 0, lastCommandMicros = 0;
uint32_t microsLast = 0; uint64_t microsHigh = 0;
bool configured = false, acquiring = false, overflowObserved = false, deadlineSkipObserved = false;
bool terminalStatusPending = false; uint8_t terminalErrorPending = 0;
uint8_t acquisitionMode = SIMULTANEOUS, adcBits = 12, channelCount = 1, analogPins[6] = {A0};
uint32_t frameRateHz = 1000, stateDwellUs = 1000; uint8_t pulseState = 0, activeBatchCount = 0;
uint8_t activeOutputMask = 0, redOutputPin = D5_RED, irOutputPin = D6_IR;
uint32_t acquisitionQueueOverflows = 0, skippedDeadlines = 0;
// Shortest time from an LED edge to the sample of that state since START.
uint32_t minimumSettleUs = 0xFFFFFFFFUL, lastLedEdgeMicros = 0;
uint64_t lastPulseStateTimestamp = 0;
bool havePulseStateTimestamp = false;
uint32_t pulseStateTimingViolations[4] = {0}, pulseStateMaximumErrorUs[4] = {0};

struct SampleRecord {
  uint32_t sequence;
  uint64_t timestamp;
  uint16_t fields[MAX_FIELDS];
};
struct TxFrame {
  uint16_t length;
  uint16_t offset;
  uint8_t bytes[MAX_TX_FRAME];
};

// Each finished batch keeps the layout it was recorded with, so a batch
// still queued when the configuration changes is encoded correctly.
struct AcquisitionBatch {
  uint8_t count;
  uint8_t fields;
  uint16_t statusFlags;
  uint32_t queueOverflows;
  uint32_t nominalPeriodUs;
  SampleRecord records[BATCH_RECORDS];
};

AcquisitionBatch acquisitionBatches[ACQUISITION_BATCH_SLOTS];
uint8_t acquisitionBatchHead = 0, acquisitionBatchesQueued = 0;
uint16_t pulseFields[MAX_FIELDS] = {0};
TxFrame txQueue[TX_QUEUE_CAPACITY];
uint8_t txHead = 0, txCount = 0;
uint16_t crcTable[256];

// CRC-16/CCITT-FALSE (polynomial 0x1021, initial 0xFFFF), one table lookup per byte.
void buildCrcTable(){for(uint16_t value=0;value<256;value++){uint16_t crc=(uint16_t)(value<<8);for(uint8_t bit=0;bit<8;bit++)crc=(crc&0x8000)?(uint16_t)((crc<<1)^0x1021):(uint16_t)(crc<<1);crcTable[value]=crc;}}
uint16_t crc16(const uint8_t* bytes, size_t length) { uint16_t crc=0xFFFF; while (length--) crc=(uint16_t)((crc<<8)^crcTable[(uint8_t)((crc>>8)^*bytes++)]); return crc; }
void writeU16(uint8_t* p,uint16_t v){p[0]=v;p[1]=v>>8;} void writeU32(uint8_t* p,uint32_t v){for(uint8_t i=0;i<4;i++)p[i]=v>>(8*i);} void writeU64(uint8_t* p,uint64_t v){for(uint8_t i=0;i<8;i++)p[i]=v>>(8*i);}
uint32_t readU32(const uint8_t* p){return (uint32_t)p[0]|((uint32_t)p[1]<<8)|((uint32_t)p[2]<<16)|((uint32_t)p[3]<<24);}
uint64_t extendedMicros(){uint32_t now=micros();if(now<microsLast)microsHigh+=(1ULL<<32);microsLast=now;return microsHigh|now;}

bool pulseMode(){return acquisitionMode==PULSEOX_4STATE||acquisitionMode==PULSEOX_SEQUENCED;}
// Fields per record and nominal record period of the current configuration.
uint8_t recordFields(){return acquisitionMode==PULSEOX_4STATE?MAX_FIELDS:acquisitionMode==PULSEOX_SEQUENCED?(uint8_t)(1+channelCount):channelCount;}
uint32_t nominalPeriodUs(){return acquisitionMode==PULSEOX_4STATE?stateDwellUs*4UL:acquisitionMode==PULSEOX_SEQUENCED?stateDwellUs:1000000UL/frameRateHz;}

void forceSafeOutputs(){ digitalWrite(D4_GREEN,LOW); digitalWrite(D5_RED,LOW); digitalWrite(D6_IR,LOW); }
uint8_t outputMask(){ return (digitalRead(D4_GREEN)==HIGH?0x01:0) | (digitalRead(D5_RED)==HIGH?0x02:0) | (digitalRead(D6_IR)==HIGH?0x04:0); }
void writeControlledOutput(uint8_t pin, uint8_t value){ if(pin==D4_GREEN||pin==D5_RED||pin==D6_IR)digitalWrite(pin,value); }
void applyActiveOutputs(){
  forceSafeOutputs();
  if(activeOutputMask&0x01)digitalWrite(D4_GREEN,HIGH);
  if(activeOutputMask&0x02)digitalWrite(D5_RED,HIGH);
  if(activeOutputMask&0x04)digitalWrite(D6_IR,HIGH);
}
void applyPulseState(uint8_t state){
  forceSafeOutputs();
  writeControlledOutput(redOutputPin, state==0 ? HIGH : LOW);
  writeControlledOutput(irOutputPin, state==2 ? HIGH : LOW);
}

bool enqueueFrame(uint8_t type,const uint8_t* payload,uint16_t length,bool sampleFrame=false){
  if(length>MAX_SAMPLE_PAYLOAD)return false;
  uint8_t queueLimit=sampleFrame ? TX_QUEUE_CAPACITY-TX_CONTROL_RESERVE : TX_QUEUE_CAPACITY;
  if(txCount>=queueLimit)return false;
  TxFrame& frame=txQueue[(txHead+txCount)%TX_QUEUE_CAPACITY];
  memcpy(frame.bytes,MAGIC,4);frame.bytes[4]=PROTOCOL_MAJOR;frame.bytes[5]=PROTOCOL_MINOR;frame.bytes[6]=type;frame.bytes[7]=0;
  writeU16(frame.bytes+8,length);writeU32(frame.bytes+10,packetSequence++);
  if(length)memcpy(frame.bytes+14,payload,length);
  writeU16(frame.bytes+14+length,crc16(frame.bytes,14+length));
  frame.length=16+length;frame.offset=0;txCount++;return true;
}
void serviceSerialTx(){
  if(!txCount)return;
  int capacity=Serial.availableForWrite();
  if(capacity<=0)return;
  TxFrame& frame=txQueue[txHead];
  uint16_t remaining=frame.length-frame.offset;
  size_t chunk=min((size_t)remaining,min((size_t)capacity,(size_t)64));
  size_t written=Serial.write(frame.bytes+frame.offset,chunk);
  frame.offset+=(uint16_t)written;
  if(frame.offset>=frame.length){txHead=(txHead+1)%TX_QUEUE_CAPACITY;txCount--;}
}
void sendHello(){uint8_t p[12];writeU32(p,FIRMWARE_BUILD);writeU32(p+4,DEVICE_ID);p[8]=1;p[9]=14;p[10]=6;p[11]=0x07;enqueueFrame(HELLO,p,sizeof(p));}
// bits: 12/14-bit ADC, six analog inputs, simultaneous + legacy 4-state +
// LED-sequenced modes (0x07), D4/D5/D6 controlled outputs, and
// 100/200/250/500/1000 Hz recommended rates.
void sendCapabilities(){uint8_t p[]={12,14,6,0x07,0x07,5,100,0,200,0,250,0,244,1,232,3};enqueueFrame(CAPABILITIES,p,sizeof(p));}
void sendError(uint8_t code){uint8_t p[]={code};enqueueFrame(ERROR_MESSAGE,p,1);}
// STATUS v0.4 carries cumulative queue overflow plus per-phase timing checks,
// then (build 0x00010007) skipped deadlines since START and the minimum
// observed LED settle in us (0xFFFFFFFF until a pulse-ox state is sampled).
void sendStatus(){
  uint8_t p[46]={0};p[0]=acquiring?1:0;p[1]=outputMask();writeU32(p+2,acquisitionQueueOverflows);
  for(uint8_t state=0;state<4;state++){writeU32(p+6+state*4,pulseStateTimingViolations[state]);writeU32(p+22+state*4,pulseStateMaximumErrorUs[state]);}
  writeU32(p+38,skippedDeadlines);writeU32(p+42,minimumSettleUs);
  enqueueFrame(STATUS,p,sizeof(p));
}

AcquisitionBatch& activeBatch(){return acquisitionBatches[(acquisitionBatchHead+acquisitionBatchesQueued)%ACQUISITION_BATCH_SLOTS];}
void resetAcquisitionBatches(){acquisitionBatchHead=0;acquisitionBatchesQueued=0;activeBatchCount=0;}
// 0x0004 marks the batch after a dropped batch; 0x0010 a batch recorded
// after one or more skipped sampling deadlines.
void finishActiveBatch(){
  if(!activeBatchCount)return;
  if(acquisitionBatchesQueued>=ACQUISITION_BATCH_SLOTS-1){acquisitionQueueOverflows++;overflowObserved=true;activeBatchCount=0;return;}
  AcquisitionBatch& slot=activeBatch();
  slot.count=activeBatchCount;slot.fields=recordFields();slot.nominalPeriodUs=nominalPeriodUs();
  slot.statusFlags=SAMPLE_FLAG_ACQUIRING|SAMPLE_FLAG_EXPLICIT_TIMESTAMPS|(overflowObserved?SAMPLE_FLAG_QUEUE_OVERFLOW:0)|(deadlineSkipObserved?SAMPLE_FLAG_DEADLINE_SKIPPED:0);
  slot.queueOverflows=acquisitionQueueOverflows;
  acquisitionBatchesQueued++;activeBatchCount=0;overflowObserved=false;deadlineSkipObserved=false;
}
void serviceBatchEncoding(){
  // Encoding and its CRC never delay a sample that is due soon.
  if(acquiring&&(int32_t)(nextTickMicros-micros())<(int32_t)ENCODE_WINDOW_US)return;
  if(!acquisitionBatchesQueued||txCount>=TX_QUEUE_CAPACITY-TX_CONTROL_RESERVE)return;
  AcquisitionBatch& slot=acquisitionBatches[acquisitionBatchHead];
  uint16_t length = 24 + (uint16_t)slot.count * 8 + (uint16_t)slot.count * slot.fields * 2;
  uint8_t p[MAX_SAMPLE_PAYLOAD];
  writeU32(p,slot.records[0].sequence);writeU64(p+4,slot.records[0].timestamp);
  writeU32(p+12,slot.nominalPeriodUs);
  p[16]=slot.fields;p[17]=slot.count;writeU16(p+18,slot.statusFlags);writeU32(p+20,slot.queueOverflows);
  uint16_t at=24;
  for(uint8_t record=0;record<slot.count;record++){writeU64(p+at,slot.records[record].timestamp);at+=8;}
  for(uint8_t record=0;record<slot.count;record++)for(uint8_t field=0;field<slot.fields;field++){writeU16(p+at,slot.records[record].fields[field]);at+=2;}
  if(!enqueueFrame(SAMPLE_BATCH,p,length,true))return;
  acquisitionBatchHead=(acquisitionBatchHead+1)%ACQUISITION_BATCH_SLOTS;acquisitionBatchesQueued--;
}
void serviceTerminalFrames(){
  if(acquisitionBatchesQueued)return;
  if(terminalErrorPending&&txCount<TX_QUEUE_CAPACITY){uint8_t code=terminalErrorPending;terminalErrorPending=0;sendError(code);}
  if(terminalStatusPending&&txCount<TX_QUEUE_CAPACITY){terminalStatusPending=false;sendStatus();}
}

void rejectConfiguration(uint8_t code){acquiring=false;configured=false;resetAcquisitionBatches();forceSafeOutputs();sendError(code);}
bool uniquePins(const uint8_t* pins,uint8_t count){for(uint8_t i=0;i<count;i++)for(uint8_t j=i+1;j<count;j++)if(pins[i]==pins[j])return false;return true;}
bool supportedFrameRate(uint32_t rate){return rate==100||rate==200||rate==250||rate==500||rate==1000;}
// Every configure function checks the whole payload before it changes any state.
bool configureSimultaneous(const uint8_t* p,uint16_t length){
  if(length<9)return false;
  uint8_t bits=p[1], count=p[6]; uint32_t rate=readU32(p+2);
  if(count<1||count>6||length!=(uint16_t)(8+count)||!(bits==12||bits==14)||!supportedFrameRate(rate))return false;
  uint8_t pins[6]; for(uint8_t i=0;i<count;i++){if(p[7+i]>5)return false;pins[i]=p[7+i];}
  if(!uniquePins(pins,count) || (p[7+count]&~0x07))return false;
  // Simultaneous mode never drives D5 red or D6 IR HIGH.
  if((p[7+count]&0x06)!=0)return false;
  acquisitionMode=SIMULTANEOUS; adcBits=bits; channelCount=count; frameRateHz=rate; activeOutputMask=p[7+count];
  for(uint8_t i=0;i<count;i++)analogPins[i]=A0+pins[i]; analogReadResolution(adcBits); return true;
}
bool configurePulseox(const uint8_t* p,uint16_t length){
  if(length!=11)return false;
  uint32_t dwell=readU32(p+2);
  if(!(p[1]==12||p[1]==14)||dwell<250||dwell>5000||p[6]!=2||p[7]>5||p[8]>5||p[7]==p[8]||p[9]<4||p[9]>6||p[10]<4||p[10]>6||p[9]==p[10])return false;
  acquisitionMode=PULSEOX_4STATE;adcBits=p[1];channelCount=2;analogPins[0]=A0+p[7];analogPins[1]=A0+p[8];stateDwellUs=dwell;activeOutputMask=0;redOutputPin=p[9];irOutputPin=p[10];analogReadResolution(adcBits);return true;
}
// Mode 2 payload: [2, bits, dwell u32, n (1..6), pin1..pinN, red, ir].
bool configurePulseSequenced(const uint8_t* p,uint16_t length){
  if(length<10)return false;
  uint8_t bits=p[1], count=p[6]; uint32_t dwell=readU32(p+2);
  if(count<1||count>6||length!=(uint16_t)(9+count)||!(bits==12||bits==14)||dwell<1000||dwell>5000)return false;
  uint8_t pins[6]; for(uint8_t i=0;i<count;i++){if(p[7+i]>5)return false;pins[i]=p[7+i];}
  uint8_t red=p[7+count], ir=p[8+count];
  if(!uniquePins(pins,count)||red<4||red>6||ir<4||ir>6||red==ir)return false;
  acquisitionMode=PULSEOX_SEQUENCED;adcBits=bits;channelCount=count;stateDwellUs=dwell;activeOutputMask=0;redOutputPin=red;irOutputPin=ir;
  for(uint8_t i=0;i<count;i++)analogPins[i]=A0+pins[i]; analogReadResolution(adcBits); return true;
}

void handleFrame(const uint8_t* frame,uint16_t length){
  if(length<16||memcmp(frame,MAGIC,4)!=0)return;
  uint16_t payloadLength=frame[8]|((uint16_t)frame[9]<<8);
  if(payloadLength>MAX_PAYLOAD||length!=14+payloadLength+2||frame[4]!=PROTOCOL_MAJOR||frame[5]!=PROTOCOL_MINOR){rejectConfiguration(1);return;}
  uint16_t received=frame[14+payloadLength]|((uint16_t)frame[15+payloadLength]<<8);
  if(crc16(frame,14+payloadLength)!=received){rejectConfiguration(2);return;}
  const uint8_t* p=frame+14;lastCommandMicros=micros();
  if(frame[6]==CONFIGURE){
    // CONFIGURE during acquisition first stops it, exactly as STOP does.
    if(acquiring){finishActiveBatch();acquiring=false;forceSafeOutputs();terminalStatusPending=true;}
    bool ok=payloadLength>=1 && ((p[0]==SIMULTANEOUS&&configureSimultaneous(p,payloadLength)) || (p[0]==PULSEOX_4STATE&&configurePulseox(p,payloadLength)) || (p[0]==PULSEOX_SEQUENCED&&configurePulseSequenced(p,payloadLength)));
    if(!ok){rejectConfiguration(3);return;} forceSafeOutputs();configured=true;enqueueFrame(CONFIG_ACK,nullptr,0);
  }else if(frame[6]==START){
    if(!configured||payloadLength){sendError(4);return;}
    forceSafeOutputs();acquiring=true;resetAcquisitionBatches();pulseState=0;recordSequence=0;acquisitionQueueOverflows=0;overflowObserved=false;deadlineSkipObserved=false;skippedDeadlines=0;minimumSettleUs=0xFFFFFFFFUL;terminalStatusPending=false;terminalErrorPending=0;havePulseStateTimestamp=false;
    for(uint8_t state=0;state<4;state++){pulseStateTimingViolations[state]=0;pulseStateMaximumErrorUs[state]=0;}lastCommandMicros=micros();
    if(pulseMode()){applyPulseState(pulseState);lastLedEdgeMicros=micros();nextTickMicros=lastLedEdgeMicros+stateDwellUs;}
    else{applyActiveOutputs();nextTickMicros=micros();}
    sendStatus();
  }else if(frame[6]==STOP){finishActiveBatch();acquiring=false;forceSafeOutputs();terminalStatusPending=true;
  }else if(frame[6]==PING){sendHello();sendCapabilities();enqueueFrame(PONG,nullptr,0);}
}

uint8_t input[14+MAX_PAYLOAD+2];uint16_t inputLength=0;
// At most COMMAND_BYTES_PER_PASS bytes per loop pass, so commands cannot
// starve sampling or the watchdog refresh.
void readCommands(){
  for(uint8_t budget=0;budget<COMMAND_BYTES_PER_PASS;budget++){
    int value=Serial.read();
    if(value<0)break;
    if(inputLength<sizeof(input))input[inputLength++]=(uint8_t)value;else{inputLength=0;rejectConfiguration(5);}
    while(inputLength>=16){
      if(memcmp(input,MAGIC,4)!=0){memmove(input,input+1,--inputLength);continue;}
      uint16_t payloadLength=input[8]|((uint16_t)input[9]<<8);uint16_t total=16+payloadLength;
      if(payloadLength>MAX_PAYLOAD){memmove(input,input+1,--inputLength);rejectConfiguration(6);continue;}
      if(inputLength<total)break;
      handleFrame(input,total);memmove(input,input+total,inputLength-total);inputLength-=total;
    }
  }
}

// A late pulse-ox state never shortens the next one below half a dwell of
// settle: a deadline already due, or closer than that to the LED edge, is
// skipped. Skipping lengthens the current state; pulseState never advances here.
void scheduleNextPulseDeadline(uint32_t ledEdge){
  uint32_t minimumSettle=stateDwellUs/2;
  while((int32_t)(micros()-nextTickMicros)>=0||(int32_t)(nextTickMicros-ledEdge)<(int32_t)minimumSettle){
    nextTickMicros+=stateDwellUs;skippedDeadlines++;deadlineSkipObserved=true;}
}
void notePulseSettle(uint32_t now){uint32_t settle=now-lastLedEdgeMicros;if(settle<minimumSettleUs)minimumSettleUs=settle;}

void acquireSimultaneous(){
  uint64_t timestamp=extendedMicros();
  AcquisitionBatch& slot=activeBatch();slot.records[activeBatchCount].sequence=recordSequence;slot.records[activeBatchCount].timestamp=timestamp;
  for(uint8_t i=0;i<channelCount;i++)slot.records[activeBatchCount].fields[i]=(uint16_t)analogRead(analogPins[i]);
  activeBatchCount++;recordSequence++;if(activeBatchCount>=BATCH_RECORDS)finishActiveBatch();
}
void observePulseStateTiming(uint8_t state,uint64_t timestamp){
  if(havePulseStateTimestamp){
    uint64_t interval=timestamp-lastPulseStateTimestamp;
    uint64_t error=interval>stateDwellUs?interval-stateDwellUs:stateDwellUs-interval;
    uint32_t boundedError=error>0xFFFFFFFFULL?0xFFFFFFFFUL:(uint32_t)error;
    if(boundedError>pulseStateMaximumErrorUs[state])pulseStateMaximumErrorUs[state]=boundedError;
    if(error>(uint64_t)stateDwellUs*5ULL/100ULL)pulseStateTimingViolations[state]++;
  }
  lastPulseStateTimestamp=timestamp;havePulseStateTimestamp=true;
}
void acquirePulseState(){
  notePulseSettle(micros());
  uint64_t stateTimestamp=extendedMicros();observePulseStateTiming(pulseState,stateTimestamp);
  uint16_t tx=(uint16_t)analogRead(analogPins[0]);uint16_t rx=(uint16_t)analogRead(analogPins[1]);
  pulseFields[pulseState]=tx;pulseFields[4+pulseState]=rx;pulseState++;
  bool cycleComplete=pulseState==4;
  if(cycleComplete){
    pulseState=0;AcquisitionBatch& slot=activeBatch();slot.records[activeBatchCount].sequence=recordSequence;slot.records[activeBatchCount].timestamp=extendedMicros();
    for(uint8_t field=0;field<MAX_FIELDS;field++)slot.records[activeBatchCount].fields[field]=pulseFields[field];
    activeBatchCount++;recordSequence++;
  }
  // Phase deadlines remain on one absolute timeline. ADC and bookkeeping time
  // therefore cannot accumulate into a periodic tenth-cycle delay.
  applyPulseState(pulseState);
  nextTickMicros+=stateDwellUs;
  lastLedEdgeMicros=micros();
  scheduleNextPulseDeadline(lastLedEdgeMicros);
  if(cycleComplete&&activeBatchCount>=BATCH_RECORDS)finishActiveBatch();
}
// One record per LED state: [state, ch1..chN], so record.sequence % 4 == state.
void acquirePulseSequenced(){
  notePulseSettle(micros());
  uint64_t timestamp=extendedMicros();observePulseStateTiming(pulseState,timestamp);
  SampleRecord& record=activeBatch().records[activeBatchCount];
  record.sequence=recordSequence;record.timestamp=timestamp;record.fields[0]=pulseState;
  for(uint8_t i=0;i<channelCount;i++)record.fields[1+i]=(uint16_t)analogRead(analogPins[i]); // LED of this state still on
  activeBatchCount++;recordSequence++;
  pulseState=(pulseState+1)&3;
  applyPulseState(pulseState);
  nextTickMicros+=stateDwellUs;
  lastLedEdgeMicros=micros();
  scheduleNextPulseDeadline(lastLedEdgeMicros);
  if(activeBatchCount>=BATCH_RECORDS)finishActiveBatch();
}

void setup(){buildCrcTable();Serial.begin(CONTROLLED_SERIAL_BAUD);pinMode(D4_GREEN,OUTPUT);pinMode(D5_RED,OUTPUT);pinMode(D6_IR,OUTPUT);forceSafeOutputs();analogReadResolution(12);lastCommandMicros=micros();sendHello();sendCapabilities();WDT.begin(HARDWARE_WATCHDOG_MS);}
void loop(){
  WDT.refresh();
  readCommands();
  if(!acquiring)forceSafeOutputs();
  else if((uint32_t)(micros()-lastCommandMicros)>COMMAND_TIMEOUT_US){acquiring=false;finishActiveBatch();forceSafeOutputs();terminalErrorPending=7;}
  else{
    uint32_t now=micros();
    if(acquisitionMode==PULSEOX_4STATE){if((int32_t)(now-nextTickMicros)>=0)acquirePulseState();}
    else if(acquisitionMode==PULSEOX_SEQUENCED){if((int32_t)(now-nextTickMicros)>=0)acquirePulseSequenced();}
    else{
      uint32_t interval=1000000UL/frameRateHz;
      if((int32_t)(now-nextTickMicros)>=0){
        acquireSimultaneous();nextTickMicros+=interval;
        uint32_t after=micros();while((int32_t)(after-nextTickMicros)>=0){deadlineSkipObserved=true;skippedDeadlines++;nextTickMicros+=interval;}
      }
    }
  }
  serviceBatchEncoding();
  serviceTerminalFrames();
  serviceSerialTx();
}
