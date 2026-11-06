// A flat C API over mGBA's GBA core, called from `src/emulator.rs`.
//
// mGBA's own API is a struct of function pointers whose layout depends on the
// features it was built with, so the Rust side talks to this shim instead of
// mirroring those structs.

#include <mgba/flags.h>

#include <mgba/core/config.h>
#include <mgba/core/core.h>
#include <mgba/core/log.h>
#include <mgba/gba/core.h>
#include <mgba-util/vfs.h>

#include <stdint.h>
#include <stdlib.h>

// mGBA writes each frame into a buffer that is 256 pixels wide, of which the
// GBA screen uses the first 240.
#define VIDEO_STRIDE 256
#define VIDEO_HEIGHT 160

struct gbamod_emulator {
	struct mCore* core;
	color_t video[VIDEO_STRIDE * VIDEO_HEIGHT];
};

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
	return emulator;
}

void gbamod_emulator_free(struct gbamod_emulator* emulator) {
	mCoreConfigDeinit(&emulator->core->config);
	emulator->core->deinit(emulator->core);
	free(emulator);
}

void gbamod_emulator_run_frames(struct gbamod_emulator* emulator, uint32_t frames) {
	for (uint32_t i = 0; i < frames; ++i) {
		emulator->core->runFrame(emulator->core);
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
