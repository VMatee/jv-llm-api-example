cmake_minimum_required(VERSION 3.16)

project(jv_llm_api_c_example LANGUAGES C)

find_package(CURL REQUIRED)
find_path(JSON_C_INCLUDE_DIR NAMES json-c/json.h REQUIRED)
find_library(JSON_C_LIBRARY NAMES json-c REQUIRED)

add_executable(jv_api_example jv_api_example.c)
target_compile_features(jv_api_example PRIVATE c_std_11)
target_include_directories(jv_api_example PRIVATE "${JSON_C_INCLUDE_DIR}")
target_link_libraries(jv_api_example PRIVATE CURL::libcurl "${JSON_C_LIBRARY}")

if(MSVC)
  target_compile_options(jv_api_example PRIVATE /W4)
else()
  target_compile_options(jv_api_example PRIVATE -Wall -Wextra -Wpedantic)
endif()
