"""Trimmed from hd2-audio-modder/const.py: only the constants the audio-patching
oracle needs.
"""

# [ToC Type]
BANK = 0
STRING = 979299457696010195
WWISE_BANK = 6006249203084351385
TEXT_BANK = 979299457696010195
WWISE_DEP = 12624162998411505776
WWISE_STREAM = 5785811756662211598
BINK_VIDEO = 6838244362054241717
# [End]

# [Hierarchy Type ID]
MUSIC_TRACK = 11
PREFETCH_STREAM = 1
SOUND = 2
STREAM = 2
# [End]

# [Plugin IDs]
VORBIS = 0x00040001
REV_AUDIO = 0x01A01052
# [End]

# [Misc]
BANK_VERSION_KEY = 0x9211BCAC
# [End]


class HircType:
    State = 0x01
    Sound = 0x02
    Action = 0x03
    Event = 0x04
    RandomSequenceContainer = 0x05
    SwitchContainer = 0x06
    ActorMixer = 0x07
    AudioBus = 0x08
    LayerContainer = 0x09
    MusicSegment = 0x0A
    MusicTrack = 0x0B
    MusicSwitch = 0x0C
    MusicRandomSequence = 0x0D
    Attenuation = 0x0E
    DialogEvent = 0x0F
    FxShareSet = 0x10
    FxCustom = 0x11
    AuxiliaryBus = 0x12
    LFO = 0x13
    Envelope = 0x14
    AudioDevice = 0x15
    TimeMod = 0x16
