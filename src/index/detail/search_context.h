// Copyright (c) 2026 Beijing Volcano Engine Technology Co., Ltd.
// SPDX-License-Identifier: Apache-2.0
#pragma once

#include <functional>
#include "index/detail/scalar/filter/filter_ops.h"
#include "index/detail/scalar/filter/sort_ops.h"

namespace vectordb {

struct SearchContext {
  FilterOpBasePtr filter_op;
  SorterOpBasePtr sorter_op;
  std::function<bool(uint64_t)> filter_out;
};

using Filter = std::function<bool(uint64_t)>;

}  // namespace vectordb
