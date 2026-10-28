#pragma once

#include <cstddef>

namespace halfcraft
{
	// Pure scheduling policy shared by the collision producer and its offline regression tests.
	// A single gather cannot be preempted; the time limit prevents starting another one after it.
	struct CollisionWorkBudget
	{
		static constexpr int         max_gathers = 3;
		static constexpr std::size_t max_queued = 24;
		static constexpr long long   frame_us = 2500;

		static constexpr bool serve_requested(unsigned ordinary_jobs) { return ordinary_jobs >= 3; }

		static constexpr bool allows(int gathered, std::size_t queued, long long elapsed_us)
		{
			return gathered < max_gathers && queued < max_queued && elapsed_us < frame_us;
		}
	};
}
