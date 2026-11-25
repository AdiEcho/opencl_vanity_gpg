#define ROTL(x, n) (((x) << (n)) | ((x) >> (32 - (n))))

#define __INJECTS__
#ifdef __INJECTS__
#define CHUNK (0)
#define FILTER(h) (false)
#define FUTURE_MODE (0)
#define COLLECT_ALL (0)
#endif

__kernel void vanity_sha1(__constant uint *hashdata, __global uint *result,
                          const ulong offset_start, const ulong offset_count,
                          const uint result_capacity) {
    uint data[CHUNK * 16];
    for (uint i = 0; i < CHUNK * 16; i++) data[i] = hashdata[i];
    uint nonce = data[1];

    for (ulong index = get_global_id(0); index < offset_count; index += get_global_size(0)) {
        ulong offset = offset_start + index;
        if (offset > 0xFFFFFFFFUL) break;

        if (FUTURE_MODE) {
            // For future mode: increment timestamp within range
            data[1] = nonce + offset;
            // Check for overflow
            if (data[1] < nonce) break;
        } else {
            // For past mode: decrement timestamp within range
            data[1] = nonce - offset;
            // Check for underflow
            if (data[1] > nonce) break;
        }

        uint h[] = {0x67452301, 0xEFCDAB89, 0x98BADCFE, 0x10325476, 0xC3D2E1F0};

        #pragma unroll
        for (uint chunk = 0; chunk < CHUNK; chunk++) {
            uint w[80];
            for (int i = 0; i < 16; i++) w[i] = data[chunk * 16 + i];
            for (int i = 16; i < 80; i++) w[i] = ROTL(w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16], 1);

            uint a = h[0];
            uint b = h[1];
            uint c = h[2];
            uint d = h[3];
            uint e = h[4];
            uint f;
            uint k;
            uint t;

            #pragma unroll
            for (int i = 0; i < 80; i++) {
                if (i < 20) {
                    f = d ^ (b & (c ^ d));
                    k = 0x5A827999;
                } else if (i < 40) {
                    f = b ^ c ^ d;
                    k = 0x6ED9EBA1;
                } else if (i < 60) {
                    f = (b & c) | (b & d) | (c & d);
                    k = 0x8F1BBCDC;
                } else {
                    f = b ^ c ^ d;
                    k = 0xCA62C1D6;
                }
                t = ROTL(a, 5) + f + e + k + w[i];
                e = d;
                d = c;
                c = ROTL(b, 30);
                b = a;
                a = t;
            }

            h[0] += a;
            h[1] += b;
            h[2] += c;
            h[3] += d;
            h[4] += e;
        }

        if (FILTER(h)) {
            #if COLLECT_ALL
            uint slot = atomic_inc(result);
            if (slot < result_capacity) result[slot + 1] = data[1];
            #else
            if (atomic_cmpxchg(result, 0u, 1u) == 0u) result[1] = data[1];
            break;
            #endif
        }
    }
}
