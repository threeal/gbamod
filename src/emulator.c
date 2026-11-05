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
