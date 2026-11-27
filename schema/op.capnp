@0x9f3c1a70b6e24d15;

# Content-addressed operation. The op id is sha256 of the canonical Cap'n Proto
# segment (the segment table is not part of the hash; it is redundant on a
# single segment). Fold order is a topological sort with ties broken by
# (lamport, op id). Wall time is stored and never used for order.
#
# The build does not run the compiler. The encoder writes this schema's
# canonical form directly. Field offsets are the ones Cap'n Proto 1.0 assigns.

struct Field {
  key @0 :Text;
  value @1 :Text;
}

struct LogLine {
  timestamp @0 :Text;
  fromState @1 :Text;
  toState @2 :Text;
  note @3 :Text;
  raw @4 :Text;
}

struct CreateBody {
  shortId @0 :Text;
  title @1 :Text;
  state @2 :Text;
  priority @3 :Text;
}

struct BallotLine {
  voter @0 :Text;
  rev @1 :Text;
  choice @2 :Text;
  stamp @3 :Text;
}

struct RevisionLine {
  revId @0 :Text;
  summary @1 :Text;
}

struct TargetLine {
  targetId @0 :Text;
  fields @1 :List(Field);
}

struct RetargetLine {
  targetId @0 :Text;
}

struct DeedLine {
  accession @0 :Text;
}

struct LinkLine {
  kind @0 :Text;
  other @1 :Text;
}

struct ResolveLine {
  field @0 :Text;
  value @1 :Text;
}

struct GenesisIssue {
  shortId @0 :Text;
  title @1 :Text;
  state @2 :Text;
  priority @3 :Text;
  properties @4 :List(Field);
  orgTags @5 :List(Text);
  statistics @6 :Text;
  propertyOrder @7 :List(Text);
  extraDrawers @8 :List(Text);
  body @9 :Text;
  logbook @10 :List(LogLine);
  lineStart @11 :UInt32;
  lineEnd @12 :UInt32;
  residue @13 :Text;
}

struct Payload {
  create @0 :CreateBody;
  setField @1 :Field;
  appendLog @2 :LogLine;
  ballot @3 :BallotLine;
  addRevision @4 :RevisionLine;
  setTarget @5 :TargetLine;
  retarget @6 :RetargetLine;
  addDeed @7 :DeedLine;
  link @8 :LinkLine;
  body @9 :Text;
  retire @10 :Text;
  resolve @11 :ResolveLine;
  genesis @12 :GenesisIssue;
}

enum Kind {
  create @0;
  setField @1;
  appendLog @2;
  ballot @3;
  addRevision @4;
  setTarget @5;
  retarget @6;
  addDeed @7;
  link @8;
  setBody @9;
  retire @10;
  resolve @11;
  genesisImport @12;
}

struct Op {
  parents @0 :List(Data);
  lamport @1 :UInt64;
  seat @2 :Text;
  project @3 :Text;
  issue @4 :Data;
  kind @5 :Kind;
  payload @6 :Payload;
  wall @7 :Int64;
}
