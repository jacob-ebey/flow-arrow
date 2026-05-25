#include <inttypes.h>
#include <stdint.h>
#include <stdio.h>

#include "stats.h"

int main(void) {
    FaStruct_JobSummary summary = score_batch(17);

    printf("jobs: 16\n");
    printf("total score: %" PRId64 "\n", summary.v_total_score);
    printf("peak score: %" PRId64 "\n", summary.v_peak_score);
    printf("total weight: %" PRId64 "\n", summary.v_total_weight);
    printf("peak weight: %" PRId64 "\n", summary.v_peak_weight);
    return 0;
}
