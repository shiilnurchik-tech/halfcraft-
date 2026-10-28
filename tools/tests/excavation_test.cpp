#include "core/hc_excavation_cells.h"
#include <cassert>
#include <cmath>
#include <iostream>
#include <random>

using namespace halfcraft::excavation;

static double area(const std::vector<Triangle>& mesh)
{
	double sum = 0;
	for (const auto& t : mesh) {
		assert(std::isfinite(areaSquared(t)));
		sum += std::sqrt(areaSquared(t));
	}
	return sum;
}

int main()
{
	const Triangle        t{ { 0, 0, 0 }, { 4, 0, 0 }, { 0, 4, 0 } };
	std::vector<Triangle> out;
	subtract(t, { { 1, 1, -1 }, { 2, 2, 1 } }, out);
	assert(std::abs(area(out) - 7) < 1e-10);
	for (const auto& f : out) {
		// Original winding survives every split.
		assert((f.b.x - f.a.x) * (f.c.y - f.a.y) - (f.b.y - f.a.y) * (f.c.x - f.a.x) > 0);
	}
	out.clear();
	subtract(t, { { -1, -1, -1 }, { 5, 5, 1 } }, out);
	assert(out.empty());
	subtract(t, { { 5, 5, 5 }, { 6, 6, 6 } }, out);
	assert(out.size() == 1 && area(out) == 8);
	out.clear();
	// A surface on an excavation boundary must disappear too.
	subtract(t, { { 0, 0, 0 }, { 4, 4, 1 } }, out);
	assert(out.empty());
	subtract(t, { { 0, 0, 0 }, { 0, 4, 1 } }, out);
	assert(out.size() == 1 && area(out) == 8);

	// Deterministic randomized squares fully inside a right triangle: exact area oracle.
	std::mt19937                           rng(42);
	std::uniform_real_distribution<double> dist(0.01, 0.9);
	for (int i = 0; i < 5000; ++i) {
		const double x = dist(rng), y = dist(rng), size = dist(rng);
		const Box    box{ { x, y, -1 }, { x + size, y + size, 1 } };
		out.clear();
		subtract(t, box, out);
		assert(std::abs(area(out) - (8 - size * size)) < 1e-10);
		std::vector<Triangle> again;
		for (const auto& f : out)
			subtract(f, box, again);
		assert(std::abs(area(out) - area(again)) < 1e-10);
	}
	// Negative coordinates and vertical faces use exactly the same axis-independent path.
	out.clear();
	subtract({ { -2, -2, -2 }, { -2, 2, -2 }, { -2, -2, 2 } }, { { -3, -1, -1 }, { -1, 0, 0 } }, out);
	assert(std::abs(area(out) - 7) < 1e-10);
	Cells cells(3);
	assert(regionAxis(-1) == -1 && regionAxis(-8) == -1 && regionAxis(-9) == -2);
	assert(cells.remove({ 7, 1, 1 }));
	assert(!cells.remove({ 7, 1, 1 }));
	assert(cells.remove({ 8, 1, 1 }));
	assert(cells.takeDirty(1).size() == 1);
	assert(cells.takeDirty(100).size() == 1);
	int  faces = 0;
	auto solid = [](Cell, int, int) { return true; };
	auto emit = [&](const std::array<Point, 4>& q, int axis, int sign) {
		++faces;
		Point u{ q[1].x - q[0].x, q[1].y - q[0].y, q[1].z - q[0].z };
		Point v{ q[2].x - q[0].x, q[2].y - q[0].y, q[2].z - q[0].z };
		Point n{ u.y * v.z - u.z * v.y, u.z * v.x - u.x * v.z, u.x * v.y - u.y * v.x };
		assert(n.axis(axis) * sign > 0);
	};
	cells.interior({ 0, 0, 0 }, solid, emit);
	cells.interior({ 1, 0, 0 }, solid, emit);
	assert(faces == 10); // Shared face across region boundary is absent on BOTH sides.
	faces = 0;
	cells.interior({ 0, 0, 0 }, [](Cell, int, int) { return false; }, emit);
	assert(faces == 0); // Unknown space is not sealed with artificial stone.
	assert(cells.remove({ 0, 0, 0 }));
	assert(!cells.remove({ 1, 0, 0 })); // Budget cannot be exceeded.
	assert(!cells.remove({ 30000000, 0, 0 }));
	cells.clear();
	assert(cells.size() == 0 && cells.takeDirty(100).empty());
	assert(cells.remove({ 0, 0, 0 }));
	out.clear();
	cells.cutSurface({ 0, 0, 0 }, { { 0, 0, 0 }, { 2, 0, 0 }, { 0, 2, 0 } }, out);
	assert(std::abs(area(out) - 1) < 1e-10);
	std::cout << "excavation: all tests passed\n";
}
