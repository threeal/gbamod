// A flat C API over mGBA's GBA core, called from `src/emulator.rs`.
//
// mGBA's own API is a struct of function pointers whose layout depends on the
// features it was built with, so the Rust side talks to this shim instead of
// mirroring those structs.

#include <mgba/flags.h>

#include <mgba/core/config.h>
#include <mgba/core/core.h>
#include <mgba/core/log.h>
#include <mgba/debugger/debugger.h>
#include <mgba/gba/core.h>
#include <mgba/internal/arm/arm.h>
#include <mgba-util/vfs.h>

#include <stdint.h>
#include <stdlib.h>

// mGBA writes each frame into a buffer that is 256 pixels wide, of which the
// GBA screen uses the first 240.
#define VIDEO_STRIDE 256
#define VIDEO_HEIGHT 160

// How many hits are kept between drains; later ones are only counted.
#define MAX_HITS 50

// How many words of the stack each hit records. Must match
// `debug::MAX_STACK_WORDS` on the Rust side.
#define STACK_WORDS 64

// What a watchpoint or breakpoint saw when it fired. Must match `RawHit` in
// `src/emulator.rs`.
struct gbamod_hit {
	int64_t point;
	uint32_t frame;
	uint32_t address;
	// 0 for a breakpoint, or the access that fired a watchpoint: 1 for a
	// write, 2 for a read.
	uint32_t access;
	uint32_t old_value;
	uint32_t new_value;
	// r0 to r15, where r15 is the address of the instruction that fired.
	uint32_t registers[16];
	uint32_t cpsr;
	uint32_t stack[STACK_WORDS];
};

struct gbamod_emulator {
	// The debugger comes first, so its callback can find the emulator.
	struct mDebugger debugger;
	struct mCore* core;
	size_t points;
	struct gbamod_hit hits[MAX_HITS];
	size_t kept_hits;
	uint32_t total_hits;
	color_t video[VIDEO_STRIDE * VIDEO_HEIGHT];
};

// Returns the address of the instruction the CPU runs next when `ahead` is 1,
// or of the one it's in the middle of when `ahead` is 2, undoing the
// pipeline's lead of two instructions in r15.
static uint32_t instruction_address(const struct ARMCore* cpu, uint32_t ahead) {
	uint32_t length = cpu->cpsr.t ? 2 : 4;
	return (uint32_t) cpu->gprs[15] - ahead * length;
}

static void record_hit(struct mDebugger* debugger, enum mDebuggerEntryReason reason, struct mDebuggerEntryInfo* info) {
	struct gbamod_emulator* emulator = (struct gbamod_emulator*) debugger;
	if (!info || (reason != DEBUGGER_ENTER_WATCHPOINT && reason != DEBUGGER_ENTER_BREAKPOINT)) {
		return;
	}
	++emulator->total_hits;
	if (emulator->kept_hits >= MAX_HITS) {
		return;
	}
	struct mCore* core = emulator->core;
	struct ARMCore* cpu = core->cpu;
	struct gbamod_hit* hit = &emulator->hits[emulator->kept_hits++];
	hit->point = info->pointId;
	hit->frame = core->frameCounter(core);
	hit->address = info->address;
	for (int i = 0; i < 15; ++i) {
		hit->registers[i] = (uint32_t) cpu->gprs[i];
	}
	hit->cpsr = cpu->cpsr.packed;
	if (reason == DEBUGGER_ENTER_WATCHPOINT) {
		hit->access = info->type.wp.accessType == WATCHPOINT_READ ? 2 : 1;
		hit->old_value = info->type.wp.oldValue;
		hit->new_value = info->type.wp.newValue;
		hit->registers[15] = instruction_address(cpu, 2);
	} else {
		hit->access = 0;
		hit->old_value = 0;
		hit->new_value = 0;
		hit->registers[15] = instruction_address(cpu, 1);
	}
	uint32_t sp = hit->registers[13];
	for (uint32_t i = 0; i < STACK_WORDS; ++i) {
		hit->stack[i] = core->rawRead32(core, sp + i * 4, -1);
	}
}

static void quiet_log(struct mLogger* logger, int category, enum mLogLevel level, const char* format, va_list args) {
	(void) logger;
	(void) category;
	(void) level;
	(void) format;
	(void) args;
}

static struct mLogger quiet_logger = { .log = quiet_log };

// Creates an emulator running a copy of the given ROM image, from power on.
// Returns NULL if mGBA can't create its core or load the ROM.
struct gbamod_emulator* gbamod_emulator_new(const void* rom, size_t size) {
	mLogSetDefaultLogger(&quiet_logger);

	struct gbamod_emulator* emulator = calloc(1, sizeof(*emulator));
	if (!emulator) {
		return NULL;
	}

	struct mCore* core = GBACoreCreate();
	if (!core) {
		free(emulator);
		return NULL;
	}
	if (!core->init(core)) {
		free(core);
		free(emulator);
		return NULL;
	}
	mCoreInitConfig(core, NULL);
	core->setVideoBuffer(core, emulator->video, VIDEO_STRIDE);

	struct VFile* vf = VFileMemChunk(rom, size);
	if (!vf || !core->loadROM(core, vf)) {
		if (vf) {
			vf->close(vf);
		}
		mCoreConfigDeinit(&core->config);
		core->deinit(core);
		free(emulator);
		return NULL;
	}

	// Keep the battery save in memory, so nothing is ever written next to the
	// ROM.
	core->loadTemporarySave(core, VFileMemChunk(NULL, 0));
	core->reset(core);

	emulator->core = core;
	emulator->debugger.type = DEBUGGER_CUSTOM;
	emulator->debugger.entered = record_hit;
	mDebuggerAttach(&emulator->debugger, core);
	return emulator;
}

void gbamod_emulator_free(struct gbamod_emulator* emulator) {
	emulator->core->detachDebugger(emulator->core);
	mCoreConfigDeinit(&emulator->core->config);
	emulator->core->deinit(emulator->core);
	free(emulator);
}

void gbamod_emulator_run_frames(struct gbamod_emulator* emulator, uint32_t frames) {
	for (uint32_t i = 0; i < frames; ++i) {
		// The debugger steps one instruction at a time to check its points,
		// so it only runs the frame when there are points to check.
		if (emulator->points) {
			mDebuggerRunFrame(&emulator->debugger);
		} else {
			emulator->core->runFrame(emulator->core);
		}
	}
}

// Returns the last frame, VIDEO_STRIDE pixels per row and VIDEO_HEIGHT rows,
// each pixel holding red, green and blue in its low three bytes.
const uint32_t* gbamod_emulator_video(const struct gbamod_emulator* emulator) {
	return (const uint32_t*) emulator->video;
}

// Sets the keys pressed while frames run, as a bit mask in the layout of the
// GBA's key register.
void gbamod_emulator_set_keys(struct gbamod_emulator* emulator, uint32_t keys) {
	emulator->core->setKeys(emulator->core, keys);
}

// Returns the number of frames run since power on.
uint32_t gbamod_emulator_frame(const struct gbamod_emulator* emulator) {
	return emulator->core->frameCounter(emulator->core);
}

// Returns the size of a savestate in bytes.
size_t gbamod_emulator_state_size(const struct gbamod_emulator* emulator) {
	return emulator->core->stateSize(emulator->core);
}

// Writes a savestate of gbamod_emulator_state_size() bytes into `state`.
// The format is mGBA's raw in-memory savestate, without a PNG wrapper.
bool gbamod_emulator_save_state(const struct gbamod_emulator* emulator, void* state) {
	return emulator->core->saveState(emulator->core, state);
}

// Restores a savestate written by gbamod_emulator_save_state(), returning
// false if its size is wrong or mGBA rejects it.
bool gbamod_emulator_load_state(struct gbamod_emulator* emulator, const void* state, size_t size) {
	if (size != emulator->core->stateSize(emulator->core)) {
		return false;
	}
	return emulator->core->loadState(emulator->core, state);
}

// Reads the byte at `address`, bypassing the side effects of a bus read.
uint8_t gbamod_emulator_read8(const struct gbamod_emulator* emulator, uint32_t address) {
	return emulator->core->rawRead8(emulator->core, address, -1);
}

// Writes `value` at `address` with a width of 1, 2, or 4 bytes, bypassing the
// side effects of a bus write.
void gbamod_emulator_write(struct gbamod_emulator* emulator, uint32_t address, uint32_t width, uint32_t value) {
	switch (width) {
	case 1:
		emulator->core->rawWrite8(emulator->core, address, -1, value);
		break;
	case 2:
		emulator->core->rawWrite16(emulator->core, address, -1, value);
		break;
	case 4:
		emulator->core->rawWrite32(emulator->core, address, -1, value);
		break;
	}
}

// Adds a watchpoint on `address` of an mWatchpointType, returning its id.
int64_t gbamod_emulator_watch(struct gbamod_emulator* emulator, uint32_t address, uint32_t type) {
	struct mWatchpoint watchpoint = { .address = address, .segment = -1, .type = type };
	struct mDebuggerPlatform* platform = emulator->debugger.platform;
	ssize_t id = platform->setWatchpoint(platform, &watchpoint);
	if (id >= 0) {
		++emulator->points;
	}
	return id;
}

// Adds a breakpoint on the instruction at `address`, returning its id.
int64_t gbamod_emulator_break(struct gbamod_emulator* emulator, uint32_t address) {
	struct mBreakpoint breakpoint = { .address = address, .segment = -1, .type = BREAKPOINT_HARDWARE };
	struct mDebuggerPlatform* platform = emulator->debugger.platform;
	ssize_t id = platform->setBreakpoint(platform, &breakpoint);
	if (id >= 0) {
		++emulator->points;
	}
	return id;
}

// Removes the watchpoint or breakpoint with the given id.
void gbamod_emulator_clear_point(struct gbamod_emulator* emulator, int64_t id) {
	struct mDebuggerPlatform* platform = emulator->debugger.platform;
	if (platform->clearBreakpoint(platform, id)) {
		--emulator->points;
	}
}

// Returns the hits kept since the last call to gbamod_emulator_clear_hits(),
// storing how many were kept in `kept` and how many fired in `total`.
const struct gbamod_hit* gbamod_emulator_hits(const struct gbamod_emulator* emulator, size_t* kept, uint32_t* total) {
	*kept = emulator->kept_hits;
	*total = emulator->total_hits;
	return emulator->hits;
}

void gbamod_emulator_clear_hits(struct gbamod_emulator* emulator) {
	emulator->kept_hits = 0;
	emulator->total_hits = 0;
}

// Writes r0 to r15 and then cpsr into `registers`, where r15 is the address
// of the instruction the CPU runs next.
void gbamod_emulator_registers(const struct gbamod_emulator* emulator, uint32_t registers[17]) {
	const struct ARMCore* cpu = emulator->core->cpu;
	for (int i = 0; i < 15; ++i) {
		registers[i] = (uint32_t) cpu->gprs[i];
	}
	registers[15] = instruction_address(cpu, 1);
	registers[16] = cpu->cpsr.packed;
}
