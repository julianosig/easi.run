// A narrow C surface over SVT-AV1 3.1.2's encoder (built from source, linked statically), so the Rust side declares four functions
// instead of mirroring the library's large configuration structure. One handle is used by one thread at a time.

#include <malloc.h>
#include <stdint.h>
#include <stdlib.h>
#include <string.h>

#include <svt-av1/EbSvtAv1Enc.h>

typedef struct {
    EbComponentType *handle;
    uint32_t height;
    int64_t pts;
} Av1;

// Starts a real-time (rtc), low-delay encoder: preset 12, no lookahead, CBR at `kbps`, BT.709 studio range (the same
// colors as the H.264 path), screen-content tools, keyframes only on request (a key picture type in low delay), AV1
// level 4.0.
int display_av1_open(Av1 **out, uint32_t width, uint32_t height, uint32_t kbps, uint32_t fps) {
    Av1 *e = calloc(1, sizeof *e);
    if (e == NULL) return -1;
    EbSvtAv1EncConfiguration cfg;
    memset(&cfg, 0, sizeof cfg);
    if (svt_av1_enc_init_handle(&e->handle, &cfg) != EB_ErrorNone) {
        free(e);
        return -1;
    }
    cfg.enc_mode = 12;
    cfg.rtc = true;
    cfg.source_width = width;
    cfg.source_height = height;
    cfg.frame_rate_numerator = fps;
    cfg.frame_rate_denominator = 1;
    cfg.encoder_bit_depth = 8;
    cfg.level = 40;
    cfg.pred_structure = SVT_AV1_PRED_LOW_DELAY_B;
    cfg.intra_period_length = -1;
    cfg.intra_refresh_type = SVT_AV1_KF_REFRESH;
    cfg.rate_control_mode = SVT_AV1_RC_MODE_CBR;
    cfg.target_bit_rate = kbps * 1000;
    cfg.look_ahead_distance = 0;
    cfg.enable_tpl_la = 0;
    cfg.screen_content_mode = 1;
    // One CPU's worth of threads: the display container has one CPU.
    cfg.level_of_parallelism = 1;
    cfg.color_primaries = EB_CICP_CP_BT_709;
    cfg.transfer_characteristics = EB_CICP_TC_BT_709;
    cfg.matrix_coefficients = EB_CICP_MC_BT_709;
    cfg.color_range = EB_CR_STUDIO_RANGE;
    if (svt_av1_enc_set_parameter(e->handle, &cfg) != EB_ErrorNone || svt_av1_enc_init(e->handle) != EB_ErrorNone) {
        svt_av1_enc_deinit_handle(e->handle);
        free(e);
        return -1;
    }
    e->height = height;
    *out = e;
    return 0;
}

// Encodes one I420 picture and copies its packet (a temporal unit of OBUs; a keyframe's carries the sequence header)
// into `out`. Returns the packet length, -1 on an encoder error, or -2 when `cap` is too small.
long display_av1_encode(Av1 *e, const uint8_t *y, const uint8_t *u, const uint8_t *v, uint32_t y_stride,
                        uint32_t uv_stride, int keyframe, uint8_t *out, size_t cap) {
    EbSvtIOFormat io;
    memset(&io, 0, sizeof io);
    io.luma = (uint8_t *)y;
    io.cb = (uint8_t *)u;
    io.cr = (uint8_t *)v;
    io.y_stride = y_stride;
    io.cb_stride = uv_stride;
    io.cr_stride = uv_stride;
    EbBufferHeaderType in;
    memset(&in, 0, sizeof in);
    in.size = sizeof in;
    in.p_buffer = (uint8_t *)&io;
    in.n_filled_len = y_stride * e->height + uv_stride * ((e->height + 1) / 2) * 2;
    in.pts = e->pts++;
    in.pic_type = keyframe ? EB_AV1_KEY_PICTURE : EB_AV1_INVALID_PICTURE;
    if (svt_av1_enc_send_picture(e->handle, &in) != EB_ErrorNone) return -1;
    // Low delay: this call blocks until the picture just sent comes out, and nothing is held back for later pictures.
    EbBufferHeaderType *packet = NULL;
    if (svt_av1_enc_get_packet(e->handle, &packet, 0) != EB_ErrorNone || packet == NULL) return -1;
    long n = packet->n_filled_len;
    if ((size_t)n > cap) n = -2;
    else memcpy(out, packet->p_buffer, (size_t)n);
    svt_av1_enc_release_out_buffer(&packet);
    return n;
}

// Returns the heap's free pages to the system (glibc keeps them otherwise), after a video encoder is released.
void display_trim_heap(void) { malloc_trim(0); }

void display_av1_close(Av1 *e) {
    if (e == NULL) return;
    // End the stream and drain it, as the library requires before deinit.
    EbBufferHeaderType eos;
    memset(&eos, 0, sizeof eos);
    eos.size = sizeof eos;
    eos.flags = EB_BUFFERFLAG_EOS;
    if (svt_av1_enc_send_picture(e->handle, &eos) == EB_ErrorNone) {
        for (int i = 0; i < 64; i++) {
            EbBufferHeaderType *packet = NULL;
            if (svt_av1_enc_get_packet(e->handle, &packet, 1) != EB_ErrorNone || packet == NULL) break;
            uint32_t flags = packet->flags;
            svt_av1_enc_release_out_buffer(&packet);
            if (flags & EB_BUFFERFLAG_EOS) break;
        }
    }
    svt_av1_enc_deinit(e->handle);
    svt_av1_enc_deinit_handle(e->handle);
    free(e);
}
