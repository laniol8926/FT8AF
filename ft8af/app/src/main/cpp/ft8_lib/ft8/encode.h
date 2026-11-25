#ifndef _INCLUDE_ENCODE_H_
#define _INCLUDE_ENCODE_H_

#include <stdint.h>

#ifdef __cplusplus
extern "C"
{
#endif

// typedef struct
// {
//     uint8_t tones[FT8_NN];
//     // for waveform readout:
//     int n_spsym;       // Number of waveform samples per symbol
//     float *pulse;      // [3 * n_spsym]
//     int idx_symbol;    // Index of the current symbol
//     float f0;          // Base frequency, Hertz
//     float signal_rate; // Waveform sample rate, Hertz
// } encoder_t;

// void encoder_init(float signal_rate, float *pulse_buffer);
// void encoder_set_f0(float f0);
// void encoder_process(const message_t *message); // in: message
// void encoder_generate(float *block);            // out: block of waveforms

/// Generate FT8 tone sequence from payload data
/// @param[in] payload - 10 byte array consisting of 77 bit payload
/// @param[out] tones  - array of FT8_NN (79) bytes to store the generated tones (encoded as 0..7)
void ft8_encode(const uint8_t* payload, uint8_t* tones);

/// Generate FT4 tone sequence from payload data
/// @param[in] payload - 10 byte array consisting of 77 bit payload
/// @param[out] tones  - array of FT4_NN (105) bytes to store the generated tones (encoded as 0..3)
void ft4_encode(const uint8_t* payload, uint8_t* tones);

/// Encode a 91-bit message (77-bit payload + 14-bit CRC, from ftx_add_crc)
/// into its full 174-bit systematic LDPC codeword. Exposed (originally
/// file-local) so tests can construct a genuine, valid codeword directly
/// -- e.g. to build controlled, deterministic LLR arrays for exercising
/// decode.c's BP/OSD/AP paths without a real waterfall at all.
/// @param[in] message  - 91 bits stored as 12 bytes (MSB first)
/// @param[out] codeword - 174 bits stored as 22 bytes (MSB first)
void encode174(const uint8_t* message, uint8_t* codeword);

#ifdef __cplusplus
}
#endif

#endif // _INCLUDE_ENCODE_H_
