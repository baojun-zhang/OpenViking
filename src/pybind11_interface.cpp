// Copyright (c) 2026 Beijing Volcano Engine Technology Co., Ltd.
// SPDX-License-Identifier: Apache-2.0
#include <pybind11/pybind11.h>
#include <pybind11/stl.h>
#include <pybind11/complex.h>
#include "index/index_engine.h"
#include "store/persist_store.h"
#include "store/volatile_store.h"
#include "common/log_utils.h"

namespace py = pybind11;
namespace vdb = vectordb;

PYBIND11_MODULE(engine, m) {
  m.def("init_logging", &vdb::init_logging, "Initialize logging");

  py::class_<vdb::AddDataRequest>(m, "AddDataRequest")
      .def(py::init<>())
      .def_readwrite("label", &vdb::AddDataRequest::label)
      .def_readwrite("vector", &vdb::AddDataRequest::vector)
      .def_readwrite("sparse_raw_terms", &vdb::AddDataRequest::sparse_raw_terms)
      .def_readwrite("sparse_values", &vdb::AddDataRequest::sparse_values)
      .def_readwrite("fields_str", &vdb::AddDataRequest::fields_str)
      .def_readwrite("old_fields_str", &vdb::AddDataRequest::old_fields_str)
      .def("__repr__", [](const vdb::AddDataRequest& p) {
        return "<AddDataRequest label=" + std::to_string(p.label) +
               ", vector=" + std::to_string(p.vector.size()) + ">";
      });

  py::class_<vdb::DeleteDataRequest>(m, "DeleteDataRequest")
      .def(py::init<>())
      .def_readwrite("label", &vdb::DeleteDataRequest::label)
      .def_readwrite("old_fields_str", &vdb::DeleteDataRequest::old_fields_str)
      .def("__repr__", [](const vdb::DeleteDataRequest& p) {
        return "<DeleteDataRequest label=" + std::to_string(p.label) +
               ", old_fields_str=" + p.old_fields_str + ">";
      });

  py::class_<vdb::SearchRequest>(m, "SearchRequest")
      .def(py::init<>())
      .def_readwrite("query", &vdb::SearchRequest::query)
      .def_readwrite("sparse_raw_terms", &vdb::SearchRequest::sparse_raw_terms)
      .def_readwrite("sparse_values", &vdb::SearchRequest::sparse_values)
      .def_readwrite("topk", &vdb::SearchRequest::topk)
      .def_readwrite("dsl", &vdb::SearchRequest::dsl)
      .def("__repr__", [](const vdb::SearchRequest& p) {
        return "<SearchRequest query=" + std::to_string(p.query.size()) +
               ", topk=" + std::to_string(p.topk) + ">";
      });

  py::class_<vdb::SearchResult>(m, "SearchResult")
      .def(py::init<>())
      .def_readwrite("result_num", &vdb::SearchResult::result_num)
      .def_readwrite("labels", &vdb::SearchResult::labels)
      .def_readwrite("scores", &vdb::SearchResult::scores)
      .def_readwrite("extra_json", &vdb::SearchResult::extra_json)
      .def("__repr__", [](const vdb::SearchResult& p) {
        return "<SearchResult result_num=" + std::to_string(p.result_num) +
               ", labels=" + std::to_string(p.labels.size()) +
               ", scores=" + std::to_string(p.scores.size()) + ">";
      });

  py::class_<vdb::FetchDataResult>(m, "FetchDatahResult")
      .def(py::init<>())
      .def_readwrite("embedding", &vdb::FetchDataResult::embedding)
      .def("__repr__", [](const vdb::FetchDataResult& p) {
        return "<FetchDataResult embedding=" +
               std::to_string(p.embedding.size()) + ">";
      });

  py::class_<vdb::StateResult>(m, "StateResult")
      .def(py::init<>())
      .def_readwrite("update_timestamp", &vdb::StateResult::update_timestamp)
      .def_readwrite("element_count", &vdb::StateResult::element_count)
      .def("__repr__", [](const vdb::StateResult& p) {
        return "<StateResult update_timestamp=" +
               std::to_string(p.update_timestamp) +
               ", element_count=" + std::to_string(p.element_count) + ">";
      });

  py::class_<vdb::IndexEngine>(m, "IndexEngine")
      .def(py::init<const std::string&>())
      .def(
          "add_data",
          [](vdb::IndexEngine& self,
             const std::vector<vdb::AddDataRequest>& data_list) {
            pybind11::gil_scoped_release release;
            return self.add_data(data_list);
          },
          "add data to index")
      .def(
          "delete_data",
          [](vdb::IndexEngine& self,
             const std::vector<vdb::DeleteDataRequest>& data_list) {
            pybind11::gil_scoped_release release;
            return self.delete_data(data_list);
          },
          "delete data from index")
      .def(
          "search",
          [](vdb::IndexEngine& self, const vdb::SearchRequest& req) {
            pybind11::gil_scoped_release release;
            return self.search(req);
          },
          "search")
      .def(
          "dump",
          [](vdb::IndexEngine& self, const std::string& dir) {
            pybind11::gil_scoped_release release;
            return self.dump(dir);
          },
          "dump index")
      .def("get_state", &vdb::IndexEngine::get_state, "get index state");

  py::class_<vdb::VolatileStore>(m, "VolatileStore")
      .def(py::init<>())
      .def("exec_op", &vdb::VolatileStore::exec_op, "exec op")
      .def(
          "get_data",
          [](vdb::VolatileStore& self, const std::vector<std::string>& keys) {
            std::vector<std::string> cxx_bin_list = self.get_data(keys);

            py::list py_bytes_list;
            for (auto& cxx_bin : cxx_bin_list) {
              py_bytes_list.append(py::bytes(cxx_bin.data(), cxx_bin.size()));
            }
            return py_bytes_list;
          },
          "get data")
      .def("delete_data", &vdb::VolatileStore::delete_data, "delete data")
      .def("put_data", &vdb::VolatileStore::put_data, "put data")
      .def("clear_data", &vdb::VolatileStore::clear_data, "clear data")
      .def(
          "seek_range",
          [](vdb::VolatileStore& self, const std::string& start_key,
             const std::string& end_key) {
            std::vector<std::pair<std::string, std::string>> cxx_kv_list =
                self.seek_range(start_key, end_key);
            py::list py_kv_list;
            for (const auto& cxx_pair : cxx_kv_list) {
              py::tuple py_pair(2);
              py_pair[0] = cxx_pair.first;
              py_pair[1] =
                  py::bytes(cxx_pair.second.data(), cxx_pair.second.size());
              py_kv_list.append(py_pair);
            }
            return py_kv_list;
          },
          "seek range");

  py::class_<vdb::PersistStore>(m, "PersistStore")
      .def(py::init<const std::string&>())
      .def(
          "exec_op",
          [](vdb::PersistStore& self, const std::vector<vdb::StorageOp>& ops) {
            pybind11::gil_scoped_release release;
            return self.exec_op(ops);
          },
          "exec op")
      .def(
          "get_data",
          [](vdb::PersistStore& self, const std::vector<std::string>& keys) {
            std::vector<std::string> cxx_bin_list;
            {
              pybind11::gil_scoped_release release;
              cxx_bin_list = self.get_data(keys);
            }

            py::list py_bytes_list;
            for (auto& cxx_bin : cxx_bin_list) {
              py_bytes_list.append(py::bytes(cxx_bin.data(), cxx_bin.size()));
            }
            return py_bytes_list;
          },
          "get data")
      .def("delete_data", &vdb::PersistStore::delete_data, "delete data")
      .def("put_data", &vdb::PersistStore::put_data, "put data")
      .def("clear_data", &vdb::PersistStore::clear_data, "clear data")
      .def(
          "seek_range",
          [](vdb::PersistStore& self, const std::string& start_key,
             const std::string& end_key) {
            std::vector<std::pair<std::string, std::string>> cxx_kv_list =
                self.seek_range(start_key, end_key);
            py::list py_kv_list;

            for (const auto& cxx_pair : cxx_kv_list) {
              py::tuple py_pair(2);
              py_pair[0] = cxx_pair.first;
              py_pair[1] =
                  py::bytes(cxx_pair.second.data(), cxx_pair.second.size());
              py_kv_list.append(py_pair);
            }
            return py_kv_list;
          },
          "seek range");

  py::enum_<vdb::StorageOp::OpType>(m, "StorageOpType")
      .value("PUT", vdb::StorageOp::OpType::PUT_OP)
      .value("DELETE", vdb::StorageOp::OpType::DELETE_OP);

  py::class_<vdb::StorageOp>(m, "StorageOp")
      .def(py::init<>())
      .def_readwrite("type", &vdb::StorageOp::type)
      .def_readwrite("key", &vdb::StorageOp::key)
      .def_readwrite("value", &vdb::StorageOp::value);
}
