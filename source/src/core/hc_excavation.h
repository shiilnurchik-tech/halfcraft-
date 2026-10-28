#pragma once

// Engine-independent surface subtraction for block-sized excavations. Coordinates must be in
// Minecraft space (convert Source coordinates with the active MapSlot first). This does NOT mutate
// a BSP, create interior stone faces, or change engine traces. See docs/excavation.md.
#include <array>
#include <cmath>
#include <vector>

namespace halfcraft
{
	namespace excavation
	{
		struct Point
		{
			double x, y, z;
			double axis(int i) const { return i == 0 ? x : (i == 1 ? y : z); }
		};

		struct Triangle
		{
			Point a, b, c;
		};

		struct Box
		{
			Point lo, hi;
		};

		// A triangle clipped by six axis planes has at most nine vertices. Fixed scratch storage
		// avoids per-plane allocations; only the caller-owned output vector may allocate.
		struct Polygon
		{
			std::array<Point, 12> points;
			int                   count = 0;
		};

		inline Polygon clip(const Polygon& in, int axis, double boundary, bool keepGreater)
		{
			Polygon out;
			if (!in.count)
				return out;
			Point  prev = in.points[in.count - 1];
			double pd = prev.axis(axis) - boundary;
			bool   pin = keepGreater ? pd >= 0 : pd <= 0;
			for (int i = 0; i < in.count; ++i) {
				const Point  cur = in.points[i];
				const double cd = cur.axis(axis) - boundary;
				const bool   cin = keepGreater ? cd >= 0 : cd <= 0;
				if (pin != cin) {
					const double t = pd / (pd - cd);
					out.points[out.count++] = { prev.x + (cur.x - prev.x) * t,
						prev.y + (cur.y - prev.y) * t, prev.z + (cur.z - prev.z) * t };
				}
				if (cin)
					out.points[out.count++] = cur;
				prev = cur;
				pd = cd;
				pin = cin;
			}
			return out;
		}

		inline double areaSquared(const Triangle& t)
		{
			const Point u{ t.b.x - t.a.x, t.b.y - t.a.y, t.b.z - t.a.z };
			const Point v{ t.c.x - t.a.x, t.c.y - t.a.y, t.c.z - t.a.z };
			const Point n{ u.y * v.z - u.z * v.y, u.z * v.x - u.x * v.z, u.x * v.y - u.y * v.x };
			return (n.x * n.x + n.y * n.y + n.z * n.z) * 0.25;
		}

		inline void triangulate(const Polygon& p, std::vector<Triangle>& out)
		{
			for (int i = 1; i + 1 < p.count; ++i) {
				Triangle t{ p.points[0], p.points[i], p.points[i + 1] };
				if (areaSquared(t) > 1e-24)
					out.push_back(t);
			}
		}

		/// Append the part of a triangle outside a closed box, preserving winding. Planes coincident
		/// with a box face are removed inside that face. Cells sharing an edge leave no duplicate area.
		/// Invalid/empty boxes are no-ops. Output is appended, never cleared.
		inline void subtract(const Triangle& triangle, const Box& box, std::vector<Triangle>& out)
		{
			for (int axis = 0; axis < 3; ++axis) {
				const double lo = box.lo.axis(axis), hi = box.hi.axis(axis);
				const double a = triangle.a.axis(axis), b = triangle.b.axis(axis), c = triangle.c.axis(axis);
				if (!(lo < hi) || (a < lo && b < lo && c < lo) || (a > hi && b > hi && c > hi)) {
					out.push_back(triangle);
					return;
				}
			}
			Polygon inside;
			inside.points[0] = triangle.a;
			inside.points[1] = triangle.b;
			inside.points[2] = triangle.c;
			inside.count = 3;
			for (int axis = 0; axis < 3 && inside.count; ++axis) {
				for (int side = 0; side < 2 && inside.count; ++side) {
					const double boundary = side ? box.hi.axis(axis) : box.lo.axis(axis);
					// Only emit an outside fragment when a vertex is strictly outside. Otherwise a
					// coplanar polygon would be emitted by both half-spaces.
					bool outside = false;
					for (int i = 0; i < inside.count; ++i)
						outside |= side ? inside.points[i].axis(axis) > boundary : inside.points[i].axis(axis) < boundary;
					if (outside)
						triangulate(clip(inside, axis, boundary, side != 0), out);
					inside = clip(inside, axis, boundary, side == 0);
				}
			}
		}
	}
}
