#include "core/hc_stream_budget.h"
#include <cassert>
#include <iostream>

int main()
{
	using Budget = halfcraft::CollisionWorkBudget;
	assert(Budget::allows(0, 0, 0));
	assert(Budget::allows(2, 23, 2499));
	assert(!Budget::allows(3, 0, 0));
	assert(!Budget::allows(0, 24, 0));
	assert(!Budget::allows(0, 0, 2500));
	assert(!Budget::allows(6, 1000, 50000));
	// Even a producer with an unlimited number of urgent updates cannot start a fourth gather.
	int done = 0;
	while (Budget::allows(done, done, 0))
		++done;
	assert(done == 3);
	// A slow gather ends this frame's work; a full worker queue prevents all new gathers.
	assert(!Budget::allows(1, 1, 10000));
	assert(!Budget::allows(0, 24, 1));
	assert(Budget::allows(0, 23, 1));
	assert(!Budget::serve_requested(0));
	assert(!Budget::serve_requested(2));
	assert(Budget::serve_requested(3));
	assert(Budget::serve_requested(100));
	std::cout << "collision work budget: all tests passed\n";
}
