#pragma once

#include "hc_excavation.h"
#include <cstdint>
#include <limits>
#include <unordered_map>
#include <unordered_set>
#include <vector>

namespace halfcraft
{
	namespace excavation
	{
		struct Cell
		{
			int  x, y, z;
			bool operator==(const Cell& b) const { return x == b.x && y == b.y && z == b.z; }
		};

		struct CellHash
		{
			std::size_t operator()(const Cell& c) const
			{
				std::uint64_t h = static_cast<std::uint32_t>(c.x);
				h = (h ^ static_cast<std::uint32_t>(c.y)) * 0x9e3779b185ebca87ULL;
				h = (h ^ static_cast<std::uint32_t>(c.z)) * 0xc2b2ae3d27d4eb4fULL;
				return static_cast<std::size_t>(h ^ (h >> 32));
			}
		};

		inline int regionAxis(int v)
		{
			const int q = v / 8;
			return q - (v % 8 < 0 ? 1 : 0);
		}

		inline Cell regionOf(Cell c)
		{
			return { regionAxis(c.x), regionAxis(c.y), regionAxis(c.z) };
		}
		inline Box cellBox(Cell c)
		{
			return { { double(c.x), double(c.y), double(c.z) }, { double(c.x) + 1, double(c.y) + 1, double(c.z) + 1 } };
		}

		/// Per-map state, owned by one thread. Clear/restore explicitly on map load or save rollback.
		/// This class is intentionally not a global: separate maps must never share excavated cells.
		class Cells
		{
		public:
			explicit Cells(std::size_t limit = 65536) : limit_(limit) {}
			bool        contains(Cell c) const { return removed_.find(c) != removed_.end(); }
			std::size_t size() const { return removed_.size(); }

			/// Caller must validate reach, tool and mineable geometry before accepting this mutation.
			/// Returns false for duplicates and limits: these must NOT issue another loot drop.
			bool remove(Cell c)
			{
				// Leave room for neighbour arithmetic and reject corrupt/extreme coordinates.
				constexpr int bound = 30000000;
				if (c.x < -bound || c.x >= bound || c.y < -bound || c.y >= bound || c.z < -bound || c.z >= bound || removed_.size() >= limit_ || !removed_.insert(c).second)
					return false;
				regions_[regionOf(c)].push_back(c);
				markDirty(c);
				return true;
			}

			void clear()
			{
				removed_.clear();
				regions_.clear();
				dirty_.clear();
			}

			/// Bounded drain: render/upload scheduler chooses its own per-frame budget.
			std::vector<Cell> takeDirty(std::size_t budget)
			{
				std::vector<Cell> result;
				result.reserve(budget < dirty_.size() ? budget : dirty_.size());
				while (budget-- && !dirty_.empty()) {
					auto it = dirty_.begin();
					result.push_back(*it);
					dirty_.erase(it);
				}
				return result;
			}

			/// Region-local candidates only. Adapter must split original surfaces into regions first;
			/// never feed a map-spanning triangle here or clip successively from a previous frame's mesh.
			void cutSurface(Cell region, const Triangle& original, std::vector<Triangle>& out) const
			{
				auto found = regions_.find(region);
				if (found == regions_.end()) {
					out.push_back(original);
					return;
				}
				std::vector<Triangle> current{ original }, next;
				for (Cell c : found->second) {
					next.clear();
					for (const auto& t : current)
						subtract(t, cellBox(c), next);
					current.swap(next);
					if (current.empty())
						break;
				}
				out.insert(out.end(), current.begin(), current.end());
			}

			/// Emit stone quads facing into the excavation. solidFace(neighbour, axis, sign) must
			/// confirm original world solid covers the WHOLE shared face; never cap air/sky/unknown.
			/// Partial faces need clipping against the original brush in the engine adapter.
			template <typename SolidFace, typename Emit>
			void interior(Cell region, SolidFace solidFace, Emit emit) const
			{
				auto found = regions_.find(region);
				if (found == regions_.end())
					return;
				for (Cell c : found->second) {
					for (int axis = 0; axis < 3; ++axis) {
						for (int sign : { -1, 1 }) {
							Cell n = c;
							(axis == 0 ? n.x : axis == 1 ? n.y
														 : n.z) += sign;
							if (contains(n) || !solidFace(n, axis, sign))
								continue;
							const int            u = (axis + 1) % 3, v = (axis + 2) % 3;
							std::array<Point, 4> quad;
							for (int i = 0; i < 4; ++i) {
								double p[3] = { double(c.x), double(c.y), double(c.z) };
								p[axis] += sign > 0 ? 1 : 0;
								p[u] += (i == 1 || i == 2) ? 1 : 0;
								p[v] += i >= 2 ? 1 : 0;
								quad[i] = { p[0], p[1], p[2] };
							}
							if (sign > 0)
								std::swap(quad[1], quad[3]);
							emit(quad, axis, -sign);
						}
					}
				}
			}

		private:
			void markDirty(Cell c)
			{
				dirty_.insert(regionOf(c));
				for (int axis = 0; axis < 3; ++axis)
					for (int sign : { -1, 1 }) {
						Cell n = c;
						(axis == 0 ? n.x : axis == 1 ? n.y
													 : n.z) += sign;
						dirty_.insert(regionOf(n));
					}
			}

			std::size_t                                           limit_;
			std::unordered_set<Cell, CellHash>                    removed_, dirty_;
			std::unordered_map<Cell, std::vector<Cell>, CellHash> regions_;
		};
	}
}
