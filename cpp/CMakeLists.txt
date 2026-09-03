cmake_minimum_required(VERSION 3.16)

project(jv_llm_api_cpp_example LANGUAGES CXX)

find_package(CURL REQUIRED)
find_package(nlohmann_json 3.2.0 REQUIRED)

add_executable(jv_api_example jv_api_example.cpp)
target_compile_features(jv_api_example PRIVATE cxx_std_17)
target_link_libraries(jv_api_example PRIVATE CURL::libcurl nlohmann_json::nlohmann_json)

if(MSVC)
  target_compile_options(jv_api_example PRIVATE /W4 /permissive-)
else()
  target_compile_options(jv_api_example PRIVATE -Wall -Wextra -Wpedantic)
endif()
